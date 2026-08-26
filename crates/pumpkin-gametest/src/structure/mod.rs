mod placement;
mod template;

pub use placement::{
    PlacedStructure, clear_structure_area, encase_structure, place_structure,
    place_structure_with_controller_rotation, remove_barriers,
};
pub use template::{StructureBlock, StructureTemplate, TestBlockMode};
