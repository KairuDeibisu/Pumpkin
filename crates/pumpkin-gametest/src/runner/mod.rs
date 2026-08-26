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

        if matches!(&self.state, TestState::Queued) {
            let placement = place_structure(
                self.world.as_ref(),
                &self.template,
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
                            Err(error) => self.state = TestState::Failed { tick: 0, error },
                        }
                    } else {
                        self.state = TestState::SettingUp { elapsed_ticks: 0 };
                    }
                }
                Err(error) => {
                    self.state = TestState::Failed { tick: 0, error };
                }
            }
            return;
        }

        match &self.state {
            TestState::SettingUp { elapsed_ticks } => {
                let elapsed_ticks = elapsed_ticks.saturating_add(1);
                if elapsed_ticks >= self.test.setup_ticks() {
                    match self.begin_running(elapsed_ticks).await {
                        Ok(()) => self.state = TestState::Running { elapsed_ticks: 0 },
                        Err(error) => {
                            self.state = TestState::Failed {
                                tick: elapsed_ticks,
                                error,
                            };
                        }
                    }
                } else {
                    self.state = TestState::SettingUp { elapsed_ticks };
                }
            }
            TestState::Running { elapsed_ticks } => {
                let tick = elapsed_ticks.saturating_add(1);
                match self.evaluate_running(tick).await {
                    Ok(RunningEvaluation::Passed) => {
                        self.successes = self.successes.saturating_add(1);
                        self.state = TestState::Passed { tick };
                    }
                    Ok(RunningEvaluation::Failed(error)) | Err(error) => {
                        self.state = TestState::Failed { tick, error };
                    }
                    Ok(RunningEvaluation::Continue) => {
                        if tick >= self.test.max_ticks() {
                            self.state = TestState::Failed {
                                tick,
                                error: GameTestError::Timeout {
                                    max_ticks: self.test.max_ticks(),
                                },
                            };
                        } else {
                            self.state = TestState::Running {
                                elapsed_ticks: tick,
                            };
                        }
                    }
                }
            }
            TestState::Queued | TestState::Passed { .. } | TestState::Failed { .. } => {}
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

        self.world.trigger_test_block(&start_blocks[0]).await
    }

    async fn evaluate_running(&self, tick: u32) -> GameTestResult<RunningEvaluation> {
        // Vanilla's TestBlock.neighborChanged triggers every non-START test block on
        // a rising redstone edge. Pumpkin's MVP processes those edges here, after
        // the normal world tick, so command/redstone changes from that tick are visible.
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

        // BlockBasedTestInstance checks ACCEPT before FAIL, so ACCEPT wins when both
        // modes are triggered during the same tick.
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

        // Vanilla re-triggers LOG blocks so they emit their message, then resets the
        // runtime-triggered bit so the same pulse is consumed only once.
        for position in self.test_block_positions(TestBlockMode::Log) {
            if self.world.test_block_triggered(&position).await? {
                self.world.trigger_test_block(&position).await?;
                self.world.reset_test_block(&position).await?;
            }
        }

        Ok(RunningEvaluation::Continue)
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
