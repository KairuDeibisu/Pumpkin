use pumpkin_util::math::position::BlockPos;
use thiserror::Error;

pub type GameTestResult<T> = Result<T, GameTestError>;

#[derive(Debug, Error)]
pub enum GameTestError {
    #[error("assertion failed at tick {tick}: {message}")]
    Assertion {
        tick: u32,
        position: Option<BlockPos>,
        message: String,
    },

    #[error("test exceeded its maximum of {max_ticks} ticks")]
    Timeout { max_ticks: u32 },

    #[error("{0}")]
    InvalidStructure(String),

    #[error("{0}")]
    World(String),
}
