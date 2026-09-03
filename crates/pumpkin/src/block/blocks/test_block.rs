use std::sync::Arc;

use pumpkin_macros::pumpkin_block;
use pumpkin_util::{GameMode, PermissionLvl};

use crate::block::entities::test_block::TestBlockBlockEntity;
use crate::block::entities::test_instance_block::TestInstanceBlockBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, EmitsRedstonePowerArgs, GetRedstonePowerArgs, NormalUseArgs,
    OnScheduledTickArgs, PlacedArgs,
};

#[pumpkin_block("minecraft:test_block")]
pub struct TestBlock;

impl BlockBehaviour for TestBlock {
    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        if args.player.permission_lvl.load() < PermissionLvl::Two {
            return BlockActionResult::Pass;
        }
        if args.player.gamemode.load() != GameMode::Creative {
            return BlockActionResult::Pass;
        }
        let Some(block_entity) = args.world.get_block_entity(args.position) else {
            return BlockActionResult::Pass;
        };
        args.world.update_block_entity(&block_entity);
        BlockActionResult::SuccessServer
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        let entity = TestBlockBlockEntity::new(*args.position);
        args.world.add_block_entity(Arc::new(entity));
    }

    fn emits_redstone_power(&self, _args: EmitsRedstonePowerArgs<'_>) -> bool {
        true
    }

    fn get_weak_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        let Some(block_entity) = args.world.get_block_entity(args.position) else {
            return 0;
        };
        let Some(test_block) = block_entity.as_any().downcast_ref::<TestBlockBlockEntity>() else {
            return 0;
        };

        if test_block.is_powered() { 15 } else { 0 }
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let Some(block_entity) = args.world.get_block_entity(args.position) else {
            return;
        };
        let Some(test_block) = block_entity.as_any().downcast_ref::<TestBlockBlockEntity>() else {
            return;
        };

        if test_block.is_powered() {
            test_block.reset(args.world);
        }
    }
}

#[pumpkin_block("minecraft:test_instance_block")]
pub struct TestInstanceBlock;

impl BlockBehaviour for TestInstanceBlock {
    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        if args.player.permission_lvl.load() < PermissionLvl::Two {
            return BlockActionResult::Pass;
        }
        if args.player.gamemode.load() != GameMode::Creative {
            return BlockActionResult::Pass;
        }
        let Some(block_entity) = args.world.get_block_entity(args.position) else {
            return BlockActionResult::Pass;
        };
        args.world.update_block_entity(&block_entity);
        BlockActionResult::SuccessServer
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        let entity = TestInstanceBlockBlockEntity::new(*args.position);
        args.world.add_block_entity(Arc::new(entity));
    }
}
