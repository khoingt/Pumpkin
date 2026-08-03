use std::sync::Arc;

use crate::block::{
    BlockBehaviour, BlockFuture, BlockMetadata, EmitsRedstonePowerArgs, GetRedstonePowerArgs,
    OnPlaceArgs, OnScheduledTickArgs, PlacedArgs,
};
use crate::world::World;
use pumpkin_data::block_properties::{
    BlockProperties, CalibratedSculkSensorLikeProperties, SculkSensorLikeProperties,
    SculkSensorPhase,
};
use pumpkin_data::{Block, BlockId, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

/// Returns the new phase after a scheduled tick, or `None` if no transition.
const fn next_phase(phase: SculkSensorPhase) -> Option<SculkSensorPhase> {
    match phase {
        SculkSensorPhase::Active => Some(SculkSensorPhase::Cooldown),
        SculkSensorPhase::Cooldown => Some(SculkSensorPhase::Inactive),
        SculkSensorPhase::Inactive => None,
    }
}

pub struct SculkSensorBlock;

impl BlockMetadata for SculkSensorBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::SCULK_SENSOR, BlockId::CALIBRATED_SCULK_SENSOR].into()
    }
}

impl SculkSensorBlock {
    pub async fn trigger(world: &Arc<World>, pos: &BlockPos, block: &Block, power: u8) {
        if block.id == BlockId::SCULK_SENSOR {
            let state = world.get_block_state(pos);
            let mut props = SculkSensorLikeProperties::from_state_id(state.id, block);
            if props.sculk_sensor_phase == SculkSensorPhase::Inactive {
                props.sculk_sensor_phase = SculkSensorPhase::Active;
                props.power = power;
                let new_state_id = props.to_state_id(block);
                world
                    .set_block_state(pos, new_state_id, BlockFlags::NOTIFY_ALL)
                    .await;
                world.update_neighbors(pos, None).await;
                world.schedule_block_tick(block, *pos, 30, TickPriority::Normal);
            }
        } else if block.id == BlockId::CALIBRATED_SCULK_SENSOR {
            let state = world.get_block_state(pos);
            let mut props = CalibratedSculkSensorLikeProperties::from_state_id(state.id, block);
            if props.sculk_sensor_phase == SculkSensorPhase::Inactive {
                props.sculk_sensor_phase = SculkSensorPhase::Active;
                props.power = power;
                world
                    .set_block_state(pos, props.to_state_id(block), BlockFlags::NOTIFY_ALL)
                    .await;
                world.update_neighbors(pos, None).await;
                world.schedule_block_tick(block, *pos, 10, TickPriority::Normal);
            }
        }
    }
}

impl BlockBehaviour for SculkSensorBlock {
    fn on_place<'a>(&'a self, args: OnPlaceArgs<'a>) -> BlockFuture<'a, BlockStateId> {
        Box::pin(async move {
            if args.block.id == BlockId::CALIBRATED_SCULK_SENSOR {
                let mut props = CalibratedSculkSensorLikeProperties::default(args.block);
                props.facing = args.player.living_entity.entity.get_horizontal_facing();
                props.to_state_id(args.block)
            } else {
                let props = SculkSensorLikeProperties::default(args.block);
                props.to_state_id(args.block)
            }
        })
    }

    fn placed<'a>(&'a self, args: PlacedArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            use crate::block::entities::calibrated_sculk_sensor::CalibratedSculkSensorBlockEntity;
            use crate::block::entities::sculk_sensor::SculkSensorBlockEntity;
            if args.block.id == BlockId::CALIBRATED_SCULK_SENSOR {
                args.world
                    .add_block_entity(Arc::new(CalibratedSculkSensorBlockEntity::new(
                        *args.position,
                    )));
            } else {
                args.world
                    .add_block_entity(Arc::new(SculkSensorBlockEntity::new(*args.position)));
            }
        })
    }

    fn emits_redstone_power<'a>(
        &'a self,
        _args: EmitsRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, bool> {
        Box::pin(async move { true })
    }

    fn get_weak_redstone_power<'a>(
        &'a self,
        args: GetRedstonePowerArgs<'a>,
    ) -> BlockFuture<'a, u8> {
        Box::pin(async move {
            if args.block.id == BlockId::SCULK_SENSOR {
                let props = SculkSensorLikeProperties::from_state_id(args.state.id, args.block);
                if props.sculk_sensor_phase == SculkSensorPhase::Active {
                    props.power
                } else {
                    0
                }
            } else if args.block.id == BlockId::CALIBRATED_SCULK_SENSOR {
                let props =
                    CalibratedSculkSensorLikeProperties::from_state_id(args.state.id, args.block);
                if props.sculk_sensor_phase == SculkSensorPhase::Active {
                    props.power
                } else {
                    0
                }
            } else {
                0
            }
        })
    }

    fn on_scheduled_tick<'a>(&'a self, args: OnScheduledTickArgs<'a>) -> BlockFuture<'a, ()> {
        Box::pin(async move {
            let state = args.world.get_block_state(args.position);
            let (new_state_id, schedule_cooldown) = if args.block.id == BlockId::SCULK_SENSOR {
                let mut props = SculkSensorLikeProperties::from_state_id(state.id, args.block);
                let Some(new_phase) = next_phase(props.sculk_sensor_phase) else {
                    return;
                };
                let cooldown = new_phase == SculkSensorPhase::Cooldown;
                props.sculk_sensor_phase = new_phase;
                props.power = 0;
                (props.to_state_id(args.block), cooldown)
            } else if args.block.id == BlockId::CALIBRATED_SCULK_SENSOR {
                let mut props =
                    CalibratedSculkSensorLikeProperties::from_state_id(state.id, args.block);
                let Some(new_phase) = next_phase(props.sculk_sensor_phase) else {
                    return;
                };
                let cooldown = new_phase == SculkSensorPhase::Cooldown;
                props.sculk_sensor_phase = new_phase;
                props.power = 0;
                (props.to_state_id(args.block), cooldown)
            } else {
                return;
            };
            args.world
                .set_block_state(args.position, new_state_id, BlockFlags::NOTIFY_ALL)
                .await;
            if schedule_cooldown {
                args.world.schedule_block_tick(
                    args.block,
                    *args.position,
                    10,
                    TickPriority::Normal,
                );
            }
            args.world.update_neighbors(args.position, None).await;
        })
    }
}
