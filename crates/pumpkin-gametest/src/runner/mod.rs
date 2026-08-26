mod state;

pub use state::TestState;

use std::sync::Arc;

use pumpkin_util::math::position::BlockPos;

use crate::block_based::BlockBasedTest;
use crate::error::{GameTestError, GameTestResult};
use crate::structure::{PlacedStructure, StructureTemplate, TestBlockMode, place_structure};
use crate::world::GameTestWorld;

enum RunningEvaluation {
    Continue,
    Passed,
    Failed(GameTestError),
}

pub struct TestRun {
    pub test: BlockBasedTest,
    pub state: TestState,
    /// Current 1-based attempt number. Vanilla increments attempts when the
    /// structure is loaded for an attempt.
    pub attempt: u32,
    pub successes: u32,
    pub placement: Option<PlacedStructure>,
    world: Arc<dyn GameTestWorld>,
    template: Arc<StructureTemplate>,
    test_x: i32,
    test_z: i32,
}

impl TestRun {
    #[must_use]
    pub fn new(
        test: BlockBasedTest,
        world: Arc<dyn GameTestWorld>,
        template: Arc<StructureTemplate>,
        test_x: i32,
        test_z: i32,
    ) -> Self {
        Self {
            test,
            state: TestState::Queued,
            attempt: 1,
            successes: 0,
            placement: None,
            world,
            template,
            test_x,
            test_z,
        }
    }

    pub async fn tick(&mut self) {
        if self.state.is_finished() {
            return;
        }

        // Move the current state out so state transitions can freely borrow `self`
        // across async calls without holding a borrow into `self.state`.
        let state = std::mem::replace(&mut self.state, TestState::Queued);
        match state {
            TestState::Queued => self.tick_queued().await,
            TestState::SettingUp { elapsed_ticks } => self.tick_setup(elapsed_ticks).await,
            TestState::Running { elapsed_ticks } => self.tick_running(elapsed_ticks).await,
            finished @ (TestState::Passed { .. } | TestState::Failed { .. }) => {
                self.state = finished;
            }
        }
    }

    async fn tick_queued(&mut self) {
        let placement = place_structure(
            self.world.as_ref(),
            &self.template,
            self.test.id(),
            self.test.rotation(),
            self.test_x,
            self.test_z,
            self.test.definition().padding,
        )
        .await;

        match placement {
            Ok(placement) => {
                self.placement = Some(placement);
                if self.test.setup_ticks() == 0 {
                    match self.begin_running(0).await {
                        Ok(()) => self.state = TestState::Running { elapsed_ticks: 0 },
                        Err(error) => self.handle_attempt_failure(0, error).await,
                    }
                } else {
                    self.state = TestState::SettingUp { elapsed_ticks: 0 };
                }
            }
            Err(error) => self.handle_attempt_failure(0, error).await,
        }
    }

    async fn tick_setup(&mut self, elapsed_ticks: u32) {
        let elapsed_ticks = elapsed_ticks.saturating_add(1);
        if elapsed_ticks < self.test.setup_ticks() {
            self.state = TestState::SettingUp { elapsed_ticks };
            return;
        }

        match self.begin_running(elapsed_ticks).await {
            Ok(()) => self.state = TestState::Running { elapsed_ticks: 0 },
            Err(error) => self.handle_attempt_failure(elapsed_ticks, error).await,
        }
    }

    async fn tick_running(&mut self, elapsed_ticks: u32) {
        let tick = elapsed_ticks.saturating_add(1);
        match self.evaluate_running(tick).await {
            Ok(RunningEvaluation::Passed) => self.handle_attempt_pass(tick).await,
            Ok(RunningEvaluation::Failed(error)) | Err(error) => {
                self.handle_attempt_failure(tick, error).await;
            }
            Ok(RunningEvaluation::Continue) => {
                // GameTestInfo times out when tickCount > timeoutTicks.
                if tick > self.test.max_ticks() {
                    self.handle_attempt_failure(
                        tick,
                        GameTestError::Timeout {
                            max_ticks: self.test.max_ticks(),
                        },
                    )
                    .await;
                } else {
                    self.state = TestState::Running {
                        elapsed_ticks: tick,
                    };
                }
            }
        }
    }

    async fn begin_running(&self, tick: u32) -> GameTestResult<()> {
        let start_blocks = self.test_block_positions(TestBlockMode::Start);
        if start_blocks.is_empty() {
            return Err(GameTestError::Assertion {
                tick,
                position: None,
                message: "missing START test block".to_string(),
            });
        }
        if start_blocks.len() != 1 {
            return Err(GameTestError::Assertion {
                tick,
                position: None,
                message: format!(
                    "expected exactly one START test block, found {}",
                    start_blocks.len()
                ),
            });
        }

        if let Some(placement) = &self.placement {
            // GameTestInfo.startTest marks the controller RUNNING immediately before
            // invoking BlockBasedTestInstance.run, which triggers START.
            self.world
                .set_test_instance_running(placement.test_instance_pos())
                .await?;
        }
        self.world.trigger_test_block(&start_blocks[0]).await
    }

    async fn evaluate_running(&self, tick: u32) -> GameTestResult<RunningEvaluation> {
        // TestBlock.neighborChanged triggers every non-START block on a rising edge.
        // This executes after Pumpkin's normal world tick, so redstone/command-block
        // changes from that tick are visible to the GameTest evaluation.
        for position in self.non_start_test_block_positions() {
            self.world.update_test_block_redstone(&position).await?;
        }

        let accept_blocks = self.test_block_positions(TestBlockMode::Accept);
        if accept_blocks.is_empty() {
            return Ok(RunningEvaluation::Failed(GameTestError::Assertion {
                tick,
                position: None,
                message: "missing ACCEPT test block".to_string(),
            }));
        }

        // Vanilla checks ACCEPT before FAIL; ACCEPT wins if both trigger this tick.
        for position in &accept_blocks {
            if self.world.test_block_triggered(position).await? {
                return Ok(RunningEvaluation::Passed);
            }
        }

        for position in self.test_block_positions(TestBlockMode::Fail) {
            if self.world.test_block_triggered(&position).await? {
                let message = self.world.test_block_message(&position).await?;
                return Ok(RunningEvaluation::Failed(GameTestError::Assertion {
                    tick,
                    position: Some(position),
                    message,
                }));
            }
        }

        for position in self.test_block_positions(TestBlockMode::Log) {
            if self.world.test_block_triggered(&position).await? {
                self.world.trigger_test_block(&position).await?;
                self.world.reset_test_block(&position).await?;
            }
        }

        Ok(RunningEvaluation::Continue)
    }

    async fn handle_attempt_pass(&mut self, tick: u32) {
        self.successes = self.successes.saturating_add(1);
        if self.successes >= self.test.required_successes() {
            if let Some(placement) = &self.placement
                && let Err(error) = self
                    .world
                    .set_test_instance_success(placement.test_instance_pos())
                    .await
            {
                self.state = TestState::Failed { tick, error };
                return;
            }
            self.state = TestState::Passed { tick };
            return;
        }

        if self.attempt < self.test.max_attempts() {
            self.queue_retry();
            return;
        }

        let error = GameTestError::ExhaustedAttempts {
            attempts: self.attempt,
            successes: self.successes,
            required_successes: self.test.required_successes(),
            last_error: "not enough successful attempts".to_string(),
        };
        self.finish_failure(tick, error, None).await;
    }

    async fn handle_attempt_failure(&mut self, tick: u32, error: GameTestError) {
        let marker = assertion_marker(&error);
        let remaining_attempts = self.test.max_attempts().saturating_sub(self.attempt);
        let can_still_reach_required =
            self.successes.saturating_add(remaining_attempts) >= self.test.required_successes();

        // ReportGameListener retries flaky failures only while the remaining number
        // of attempts can still reach requiredSuccesses.
        if self.attempt < self.test.max_attempts() && can_still_reach_required {
            self.queue_retry();
            return;
        }

        let final_error = if self.test.max_attempts() > 1 {
            GameTestError::ExhaustedAttempts {
                attempts: self.attempt,
                successes: self.successes,
                required_successes: self.test.required_successes(),
                last_error: error.to_string(),
            }
        } else {
            error
        };
        self.finish_failure(tick, final_error, marker).await;
    }

    fn queue_retry(&mut self) {
        self.attempt = self.attempt.saturating_add(1);
        self.placement = None;
        self.state = TestState::Queued;
    }

    async fn finish_failure(
        &mut self,
        tick: u32,
        error: GameTestError,
        marker: Option<(BlockPos, String)>,
    ) {
        if let Some(placement) = &self.placement {
            let message = error.to_string();
            if let Err(controller_error) = self
                .world
                .set_test_instance_failure(
                    placement.test_instance_pos(),
                    &message,
                    marker,
                )
                .await
            {
                self.state = TestState::Failed {
                    tick,
                    error: controller_error,
                };
                return;
            }
        }
        self.state = TestState::Failed { tick, error };
    }

    fn test_block_positions(&self, mode: TestBlockMode) -> Vec<BlockPos> {
        let Some(placement) = &self.placement else {
            return Vec::new();
        };

        self.template
            .blocks()
            .iter()
            .filter(|block| block.test_mode == Some(mode))
            .map(|block| {
                placement.transform(&BlockPos::new(
                    block.position[0],
                    block.position[1],
                    block.position[2],
                ))
            })
            .collect()
    }

    fn non_start_test_block_positions(&self) -> Vec<BlockPos> {
        let Some(placement) = &self.placement else {
            return Vec::new();
        };

        self.template
            .blocks()
            .iter()
            .filter(|block| block.test_mode.is_some_and(|mode| mode != TestBlockMode::Start))
            .map(|block| {
                placement.transform(&BlockPos::new(
                    block.position[0],
                    block.position[1],
                    block.position[2],
                ))
            })
            .collect()
    }
}

fn assertion_marker(error: &GameTestError) -> Option<(BlockPos, String)> {
    match error {
        GameTestError::Assertion {
            position: Some(position),
            message,
            ..
        } => Some((*position, message.clone())),
        _ => None,
    }
}

#[derive(Default)]
pub struct TestRunner {
    active: Vec<TestRun>,
}

impl TestRunner {
    #[must_use]
    pub const fn new() -> Self {
        Self { active: Vec::new() }
    }

    pub fn enqueue(&mut self, run: TestRun) {
        self.active.push(run);
    }

    pub async fn tick(&mut self) {
        for run in &mut self.active {
            run.tick().await;
        }
    }

    #[must_use]
    pub fn active(&self) -> &[TestRun] {
        &self.active
    }

    pub fn active_mut(&mut self) -> &mut [TestRun] {
        &mut self.active
    }
}
