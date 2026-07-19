//! Vanilla `net.minecraft.world.level.gameevent.vibrations.*`

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use pumpkin_data::block_properties::{
    BlockProperties, CalibratedSculkSensorLikeProperties, SculkSensorLikeProperties,
    SculkSensorPhase,
};
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::{BlockId, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use tokio::sync::Mutex;

use crate::entity::EntityBase;
use crate::world::World;

use super::dist_sq;

// ---------------------------------------------------------------------------
// GameEventExt — frequency mapping (Vanilla VIBRATION_FREQUENCY_FOR_EVENT)
// ---------------------------------------------------------------------------

pub trait GameEventExt {
    fn default_frequency(&self) -> u32;
}

impl GameEventExt for GameEvent {
    fn default_frequency(&self) -> u32 {
        match self {
            // Freq 1
            Self::Step | Self::Swim | Self::Flap | Self::Resonate1 => 1,
            // Freq 2
            Self::ProjectileLand | Self::HitGround | Self::Splash | Self::Bounce | Self::Resonate2 => 2,
            // Freq 3
            Self::ItemInteractFinish | Self::ProjectileShoot | Self::InstrumentPlay | Self::Resonate3 => 3,
            // Freq 4
            Self::EntityAction | Self::ElytraGlide | Self::Unequip | Self::Resonate4 => 4,
            // Freq 5
            Self::EntityDismount | Self::Equip | Self::Resonate5 => 5,
            // Freq 6
            Self::EntityInteract | Self::Shear | Self::EntityMount | Self::Resonate6 => 6,
            // Freq 7
            Self::EntityDamage | Self::Resonate7 => 7,
            // Freq 8
            Self::Drink | Self::Eat | Self::Resonate8 => 8,
            // Freq 9
            Self::ContainerClose
            | Self::BlockClose
            | Self::BlockDeactivate
            | Self::BlockDetach
            | Self::Resonate9 => 9,
            // Freq 10
            Self::ContainerOpen
            | Self::BlockOpen
            | Self::BlockActivate
            | Self::BlockAttach
            | Self::PrimeFuse
            | Self::NoteBlockPlay
            | Self::Resonate10 => 10,
            // Freq 11
            Self::BlockChange | Self::Resonate11 => 11,
            // Freq 12
            Self::BlockDestroy | Self::FluidPickup | Self::Resonate12 => 12,
            // Freq 13
            Self::BlockPlace | Self::FluidPlace | Self::Resonate13 => 13,
            // Freq 14
            Self::EntityPlace | Self::LightningStrike | Self::Teleport | Self::Resonate14 => 14,
            // Freq 15
            Self::EntityDie | Self::Explode | Self::Resonate15 => 15,
            _ => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// getRedstoneStrengthForDistance — vanilla line 118-121
// ---------------------------------------------------------------------------

#[must_use]
pub fn get_redstone_strength_for_distance(d: f32, listener_radius: i32) -> i32 {
    if listener_radius == 0 {
        return 0;
    }
    let power_scale = 15.0 / (listener_radius as f32);
    (15 - (power_scale * d).floor() as i32).max(1)
}

// ---------------------------------------------------------------------------
// GameEventContext
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct GameEventContext {
    source_entity: Option<Arc<dyn EntityBase>>,
}

impl GameEventContext {
    pub fn of_entity(entity: &Arc<dyn EntityBase>) -> Self {
        Self {
            source_entity: Some(Arc::clone(entity)),
        }
    }
    #[must_use]
    pub fn source_entity(&self) -> Option<&Arc<dyn EntityBase>> {
        self.source_entity.as_ref()
    }
}

// ---------------------------------------------------------------------------
// VibrationInfo
// ---------------------------------------------------------------------------

pub struct VibrationInfo {
    pub game_event: GameEvent,
    pub pos: Vector3<f64>,
    pub source_entity: Option<Arc<dyn EntityBase>>,
    pub distance: f32,
    pub tick: i64,
}

// ---------------------------------------------------------------------------
// VibrationSelector — single-candidate, same-tick replacement (vanilla model)
// ---------------------------------------------------------------------------

pub struct VibrationSelector {
    candidate: Option<VibrationInfo>,
}

impl Default for VibrationSelector {
    fn default() -> Self {
        Self::new()
    }
}
impl VibrationSelector {
    #[must_use]
    pub const fn new() -> Self {
        Self { candidate: None }
    }

    /// Vanilla `addCandidate` — accept or replace based on distance/frequency.
    pub fn add_candidate(&mut self, info: VibrationInfo) {
        let should_replace = match &self.candidate {
            None => true,
            Some(prev) => {
                if info.tick != prev.tick {
                    false
                } else if info.distance < prev.distance {
                    true
                } else if info.distance > prev.distance {
                    false
                } else {
                    // Same distance — higher frequency wins tiebreak.
                    info.game_event.default_frequency()
                        > prev.game_event.default_frequency()
                }
            }
        };
        if should_replace {
            self.candidate = Some(info);
        }
    }

    /// Vanilla `chosenCandidate` — returns the vibration only if it arrived
    /// at least one tick ago (`tick < time`), then clears the candidate.
    pub fn choose(&mut self, time: i64) -> Option<VibrationInfo> {
        let info = self.candidate.as_ref()?;
        if info.tick < time {
            self.candidate.take()
        } else {
            None
        }
    }

    /// Vanilla `startOver` — clear the stored candidate.
    pub fn start_over(&mut self) {
        self.candidate = None;
    }
}

// ---------------------------------------------------------------------------
// VibrationData
// ---------------------------------------------------------------------------

pub struct VibrationData {
    current_vibration: Option<VibrationInfo>,
    receive_time: i32,
    selector: VibrationSelector,
}

impl Default for VibrationData {
    fn default() -> Self {
        Self::new()
    }
}
impl VibrationData {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            current_vibration: None,
            receive_time: 0,
            selector: VibrationSelector::new(),
        }
    }

    pub const fn selector_mut(&mut self) -> &mut VibrationSelector {
        &mut self.selector
    }

    /// Vanilla `data.getCurrentVibration() != null`.
    #[must_use]
    pub const fn has_current_vibration(&self) -> bool {
        self.current_vibration.is_some()
    }

    pub fn try_select_and_schedule(&mut self, world_tick: i64, user: &dyn VibrationUser) {
        if self.current_vibration.is_some() {
            return;
        }
        let Some(vib) = self.selector.choose(world_tick) else {
            return;
        };
        self.receive_time = user.calculate_travel_time_in_ticks(vib.distance);
        self.current_vibration = Some(vib);
    }

    pub const fn tick_receive(&mut self) -> bool {
        if self.receive_time > 0 {
            self.receive_time -= 1;
        }
        self.receive_time == 0 && self.current_vibration.is_some()
    }

    pub const fn consume_current(&mut self) -> Option<VibrationInfo> {
        self.current_vibration.take()
    }
}

// ---------------------------------------------------------------------------
// VibrationUser
// ---------------------------------------------------------------------------

pub trait VibrationUser: Send + Sync {
    fn get_listener_radius(&self) -> i32;

    fn can_receive_vibration(
        &self,
        world: &Arc<World>,
        listener_pos: &BlockPos,
        event: &GameEvent,
        context: &GameEventContext,
    ) -> bool;

    fn on_receive_vibration<'a>(
        &'a self,
        world: &'a Arc<World>,
        listener_pos: &'a BlockPos,
        event: &'a GameEvent,
        context: &'a GameEventContext,
        receiving_distance: f32,
        source_entity: Option<&'a Arc<dyn EntityBase>>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

    fn calculate_travel_time_in_ticks(&self, distance: f32) -> i32 {
        distance.floor() as i32
    }
}

// ---------------------------------------------------------------------------
// VibrationListener
// ---------------------------------------------------------------------------

pub struct VibrationListener {
    pub position: BlockPos,
    pub data: Mutex<VibrationData>,
}

impl VibrationListener {
    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            data: Mutex::new(VibrationData::new()),
        }
    }

    /// Vanilla `Listener.handleGameEvent` — validate + schedule candidate.
    pub async fn handle_game_event(
        &self,
        world: &Arc<World>,
        event: GameEvent,
        context: &GameEventContext,
        source_position: &Vector3<f64>,
        user: &dyn VibrationUser,
    ) -> bool {
        let listener_center = Vector3::new(
            f64::from(self.position.0.x) + 0.5,
            f64::from(self.position.0.y) + 0.5,
            f64::from(self.position.0.z) + 0.5,
        );
        let d_sq = dist_sq(source_position, &listener_center);
        let r = user.get_listener_radius();
        if d_sq > f64::from(r * r) {
            return false;
        }
        if !user.can_receive_vibration(world, &self.position, &event, context) {
            return false;
        }

        let distance = d_sq.sqrt() as f32;
        let world_tick = world.level_time.lock().await.query_gametime();

        let mut data = self.data.lock().await;
        // Vanilla: reject new candidates while a vibration is in flight
        // (VibrationSystem.java:213 — `data.getCurrentVibration() != null`).
        if data.has_current_vibration() {
            return false;
        }
        data.selector_mut().add_candidate(VibrationInfo {
            game_event: event,
            pos: *source_position,
            source_entity: context.source_entity().cloned(),
            distance,
            tick: world_tick,
        });
        true
    }
}

// ---------------------------------------------------------------------------
// VibrationTicker
// ---------------------------------------------------------------------------

pub struct VibrationTicker;

impl VibrationTicker {
    pub async fn tick(
        world: &Arc<World>,
        listener: &VibrationListener,
        user: &dyn VibrationUser,
    ) {
        // Batch all three locks into one acquisition — avoids 3 async await points
        // per BE-tick and the lock reacquire dance.
        let arrived_vib = {
            let mut data = listener.data.lock().await;
            let world_tick = world.level_time.lock().await.query_gametime();
            data.try_select_and_schedule(world_tick, user);
            if data.tick_receive() {
                data.consume_current()
            } else {
                None
            }
        };
        let Some(vib) = arrived_vib else { return };

        user.on_receive_vibration(
            world,
            &listener.position,
            &vib.game_event,
            &GameEventContext::default(),
            vib.distance,
            vib.source_entity.as_ref(),
        )
        .await;
    }
}

// ---------------------------------------------------------------------------
// Helpers — sculk sensor phase checks
// ---------------------------------------------------------------------------

fn is_sculk_sensor_block(state_id: BlockStateId) -> bool {
    let id = state_id.to_block().id;
    id == BlockId::SCULK_SENSOR || id == BlockId::CALIBRATED_SCULK_SENSOR
}

fn is_phase_inactive(state_id: BlockStateId) -> bool {
    let block = state_id.to_block();
    if block.id == BlockId::SCULK_SENSOR {
        let props = SculkSensorLikeProperties::from_state_id(state_id, block);
        props.sculk_sensor_phase == SculkSensorPhase::Inactive
    } else if block.id == BlockId::CALIBRATED_SCULK_SENSOR {
        let props = CalibratedSculkSensorLikeProperties::from_state_id(state_id, block);
        props.sculk_sensor_phase == SculkSensorPhase::Inactive
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// SculkSensorVibrationUser — shared between regular & calibrated sensors
// ---------------------------------------------------------------------------

pub struct SculkSensorVibrationUser {
    pub position: BlockPos,
    pub radius: i32,
}

impl SculkSensorVibrationUser {
    #[must_use]
    pub const fn new(position: BlockPos, radius: i32) -> Self {
        Self { position, radius }
    }
}

impl VibrationUser for SculkSensorVibrationUser {
    fn get_listener_radius(&self) -> i32 {
        self.radius
    }

    fn can_receive_vibration(
        &self,
        world: &Arc<World>,
        listener_pos: &BlockPos,
        event: &GameEvent,
        _context: &GameEventContext,
    ) -> bool {
        let state = world.get_block_state(listener_pos);
        if !is_sculk_sensor_block(state.id) {
            return false;
        }
        if !is_phase_inactive(state.id) {
            return false;
        }
        if event.default_frequency() == 0 {
            return false;
        }
        // ponytail: IGNORE_VIBRATIONS_SNEAKING filter deferred
        true
    }
    fn on_receive_vibration<'a>(
        &'a self,
        world: &'a Arc<World>,
        listener_pos: &'a BlockPos,
        event: &'a GameEvent,
        _context: &'a GameEventContext,
        receiving_distance: f32,
        _source_entity: Option<&'a Arc<dyn EntityBase>>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        use crate::block::blocks::redstone::sculk_sensor::SculkSensorBlock;

        Box::pin(async move {
            let state = world.get_block_state(listener_pos);
            if !is_phase_inactive(state.id) {
                return;
            }

            let event_frequency = event.default_frequency() as i32;
            let power = get_redstone_strength_for_distance(receiving_distance, self.radius);
            let block = state.id.to_block();

            if let Some(be) = world.get_block_entity(listener_pos) {
                use crate::block::entities::sculk_sensor::SculkSensorBlockEntity;
                if let Some(sensor) = be.as_any().downcast_ref::<SculkSensorBlockEntity>() {
                    *sensor.last_vibration_frequency.lock().await = event_frequency;
                }
            }
            SculkSensorBlock::trigger(world, listener_pos, block, power as u8).await;
        })
    }
}
