mod state;

pub use state::TestState;

use crate::block_based::BlockBasedTest;
use crate::structure::PlacedStructure;

#[derive(Debug)]
pub struct TestRun {
    pub test: BlockBasedTest,
    pub state: TestState,
    pub attempt: u32,
    pub successes: u32,
    pub placement: Option<PlacedStructure>,
}

impl TestRun {
    #[must_use]
    pub const fn new(test: BlockBasedTest) -> Self {
        Self {
            test,
            state: TestState::Queued,
            attempt: 1,
            successes: 0,
            placement: None,
        }
    }
}

#[derive(Debug, Default)]
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

    #[must_use]
    pub fn active(&self) -> &[TestRun] {
        &self.active
    }

    pub fn active_mut(&mut self) -> &mut [TestRun] {
        &mut self.active
    }
}
