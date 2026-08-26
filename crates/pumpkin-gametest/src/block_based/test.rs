use crate::model::TestDefinition;

#[derive(Clone, Debug)]
pub struct BlockBasedTest {
    id: String,
    definition: TestDefinition,
}

impl BlockBasedTest {
    #[must_use]
    pub fn new(id: impl Into<String>, definition: TestDefinition) -> Self {
        Self {
            id: id.into(),
            definition,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn definition(&self) -> &TestDefinition {
        &self.definition
    }

    #[must_use]
    pub const fn max_ticks(&self) -> u32 {
        self.definition.max_ticks as u32
    }

    #[must_use]
    pub const fn setup_ticks(&self) -> u32 {
        self.definition.setup_ticks as u32
    }
}
