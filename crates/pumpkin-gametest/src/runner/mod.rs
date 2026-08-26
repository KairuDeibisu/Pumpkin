mod state;

pub use state::TestState;

use std::sync::Arc;

use crate::block_based::BlockBasedTest;
use crate::error::GameTestError;
use crate::structure::{PlacedStructure, StructureTemplate, place_structure};
use crate::world::GameTestWorld;

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
                    self.state = if self.test.setup_ticks() == 0 {
                        TestState::Running { elapsed_ticks: 0 }
                    } else {
                        TestState::SettingUp { elapsed_ticks: 0 }
                    };
                }
                Err(error) => {
                    self.state = TestState::Failed { tick: 0, error };
                }
            }
            return;
        }

        let next_state = match &self.state {
            TestState::SettingUp { elapsed_ticks } => {
                let elapsed_ticks = elapsed_ticks.saturating_add(1);
                if elapsed_ticks >= self.test.setup_ticks() {
                    TestState::Running { elapsed_ticks: 0 }
                } else {
                    TestState::SettingUp { elapsed_ticks }
                }
            }
            TestState::Running { elapsed_ticks } => {
                let elapsed_ticks = elapsed_ticks.saturating_add(1);
                if elapsed_ticks >= self.test.max_ticks() {
                    TestState::Failed {
                        tick: elapsed_ticks,
                        error: GameTestError::Timeout {
                            max_ticks: self.test.max_ticks(),
                        },
                    }
                } else {
                    TestState::Running { elapsed_ticks }
                }
            }
            TestState::Queued | TestState::Passed { .. } | TestState::Failed { .. } => return,
        };

        self.state = next_state;
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
