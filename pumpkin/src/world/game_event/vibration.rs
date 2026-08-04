//! Vanilla `net.minecraft.world.level.gameevent.vibrations.*`

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use pumpkin_data::block_properties::{
    BlockProperties, CalibratedSculkSensorLikeProperties, SculkSensorLikeProperties,
    SculkSensorPhase,
};
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::{BlockId, BlockStateId, particle::Particle};
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_protocol::java::client::play::CParticle;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use crate::entity::EntityBase;
use crate::world::World;

pub trait GameEventExt {
    fn default_frequency(&self) -> u32;
}

impl GameEventExt for GameEvent {
    fn default_frequency(&self) -> u32 {
        match self {
            Self::Step | Self::Swim | Self::Flap | Self::Resonate1 => 1,
            Self::ProjectileLand
            | Self::HitGround
            | Self::Splash
            | Self::Bounce
            | Self::Resonate2 => 2,
            Self::ItemInteractFinish
            | Self::ProjectileShoot
            | Self::InstrumentPlay
            | Self::Resonate3 => 3,
            Self::EntityAction | Self::ElytraGlide | Self::Unequip | Self::Resonate4 => 4,
            Self::EntityDismount | Self::Equip | Self::Resonate5 => 5,
            Self::EntityInteract | Self::Shear | Self::EntityMount | Self::Resonate6 => 6,
            Self::EntityDamage | Self::Resonate7 => 7,
            Self::Drink | Self::Eat | Self::Resonate8 => 8,
            Self::ContainerClose
            | Self::BlockClose
            | Self::BlockDeactivate
            | Self::BlockDetach
            | Self::Resonate9 => 9,
            Self::ContainerOpen
            | Self::BlockOpen
            | Self::BlockActivate
            | Self::BlockAttach
            | Self::PrimeFuse
            | Self::NoteBlockPlay
            | Self::Resonate10 => 10,
            Self::BlockChange | Self::Resonate11 => 11,
            Self::BlockDestroy | Self::FluidPickup | Self::Resonate12 => 12,
            Self::BlockPlace | Self::FluidPlace | Self::Resonate13 => 13,
            Self::EntityPlace | Self::LightningStrike | Self::Teleport | Self::Resonate14 => 14,
            Self::EntityDie | Self::Explode | Self::Resonate15 => 15,
            _ => 0,
        }
    }
}

#[must_use]
pub fn get_redstone_strength_for_distance(d: f32, listener_radius: i32) -> i32 {
    if listener_radius == 0 {
        return 0;
    }
    let power_scale = 15.0 / (listener_radius as f32);
    (15 - (power_scale * d).floor() as i32).max(1)
}

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

pub struct VibrationInfo {
    pub game_event: GameEvent,
    pub pos: Vector3<f64>,
    pub source_entity: Option<Arc<dyn EntityBase>>,
    pub distance: f32,
    pub tick: i64,
}

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
                    info.game_event.default_frequency() > prev.game_event.default_frequency()
                }
            }
        };
        if should_replace {
            self.candidate = Some(info);
        }
    }

    pub fn choose(&mut self, time: i64) -> Option<VibrationInfo> {
        let info = self.candidate.as_ref()?;
        if info.tick < time {
            self.candidate.take()
        } else {
            None
        }
    }
}

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

    #[must_use]
    pub const fn has_current_vibration(&self) -> bool {
        self.current_vibration.is_some()
    }

    pub fn try_select_and_schedule(
        &mut self,
        world_tick: i64,
        user: &dyn VibrationUser,
    ) -> Option<(Vector3<f64>, i32)> {
        if self.current_vibration.is_some() {
            return None;
        }
        let vib = self.selector.choose(world_tick)?;
        self.receive_time = user.calculate_travel_time_in_ticks(vib.distance);
        let particle = (vib.pos, self.receive_time);
        self.current_vibration = Some(vib);
        Some(particle)
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

pub trait VibrationUser: Send + Sync {
    fn get_listener_radius(&self) -> i32;

    fn can_receive_vibration(
        &self,
        world: &Arc<World>,
        listener_pos: &BlockPos,
        source_pos: &BlockPos,
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

pub struct VibrationListener {
    pub position: BlockPos,
    pub data: Mutex<VibrationData>,
}

impl VibrationListener {
    #[must_use]
    pub const fn new(position: BlockPos) -> Self {
        Self {
            position,
            data: Mutex::new(VibrationData::new()),
        }
    }

    pub fn handle_game_event(
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
        let d_sq = source_position.squared_distance_to_vec(&listener_center);
        let r = user.get_listener_radius();
        if d_sq > f64::from(r * r) {
            return false;
        }
        if !user.can_receive_vibration(
            world,
            &self.position,
            &BlockPos::floored_v(*source_position),
            &event,
            context,
        ) {
            return false;
        }

        let distance = d_sq.sqrt() as f32;
        let world_tick = world.game_time.load(Ordering::Relaxed);
        let mut data = self.data.lock().unwrap();
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

pub async fn vibration_tick(
    world: &Arc<World>,
    listener: &VibrationListener,
    user: &dyn VibrationUser,
) {
    let world_tick = world.game_time.load(Ordering::Relaxed);
    let (particle, vib) = {
        let mut data = listener.data.lock().unwrap();
        let particle = data.try_select_and_schedule(world_tick, user);
        let vibration = if data.tick_receive() {
            data.consume_current()
        } else {
            None
        };
        (particle, vibration)
    };

    if let Some((origin, arrival_in_ticks)) = particle {
        let data = vibration_particle_data(listener.position, arrival_in_ticks);
        let packet = CParticle::new(
            false,
            false,
            origin,
            Vector3::new(0.0, 0.0, 0.0),
            0.0,
            1,
            VarInt(Particle::Vibration as i32),
            &data,
        );
        for player in world
            .players
            .load()
            .iter()
            .filter(|player| player.position().squared_distance_to_vec(&origin) <= 1024.0)
        {
            player.client.try_enqueue_packet(&packet);
        }
    }

    let Some(vib) = vib else { return };

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

fn vibration_particle_data(destination: BlockPos, arrival_in_ticks: i32) -> Vec<u8> {
    let arrival_in_ticks = VarInt(arrival_in_ticks);
    let mut data = Vec::with_capacity(9 + arrival_in_ticks.written_size());
    data.push(0); // minecraft:block position source
    data.extend_from_slice(&destination.as_long().to_be_bytes());
    arrival_in_ticks
        .encode(&mut data)
        .expect("writing a VarInt to Vec cannot fail");
    data
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
        source_pos: &BlockPos,
        event: &GameEvent,
        _context: &GameEventContext,
    ) -> bool {
        if source_pos == listener_pos
            && matches!(event, GameEvent::BlockPlace | GameEvent::BlockDestroy)
        {
            return false;
        }
        let state = world.get_block_state(listener_pos);
        if !is_phase_inactive(state.id) {
            return false;
        }
        if event.default_frequency() == 0 {
            return false;
        }
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
                use crate::block::entities::calibrated_sculk_sensor::CalibratedSculkSensorBlockEntity;
                use crate::block::entities::sculk_sensor::SculkSensorBlockEntity;
                if let Some(sensor) = be.as_any().downcast_ref::<SculkSensorBlockEntity>() {
                    sensor
                        .last_vibration_frequency
                        .store(event_frequency, Ordering::Relaxed);
                } else if let Some(sensor) = be
                    .as_any()
                    .downcast_ref::<CalibratedSculkSensorBlockEntity>()
                {
                    sensor
                        .last_vibration_frequency
                        .store(event_frequency, Ordering::Relaxed);
                }
                world.update_block_entity(&be);
            }
            SculkSensorBlock::trigger(world, listener_pos, block, power as u8).await;
        })
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_util::{math::position::BlockPos, math::vector3::Vector3};

    use super::vibration_particle_data;

    #[test]
    fn vibration_particle_encodes_block_destination_and_arrival() {
        assert_eq!(
            vibration_particle_data(BlockPos(Vector3::new(0, 0, 0)), 300),
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0xAC, 0x02]
        );
    }
}
