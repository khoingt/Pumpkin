use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use dashmap::DashMap;
use pumpkin::entity::EntityBase;
use pumpkin::world::World;
use pumpkin::world::game_event::vibration::{
    GameEventContext, VibrationData, VibrationInfo, VibrationSelector, VibrationUser,
    get_redstone_strength_for_distance,
};
use pumpkin_data::game_event::GameEvent;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::math::vector3::Vector3;
use rustc_hash::FxHashSet;
use std::hint::black_box;

// ---------------------------------------------------------------------------
// Minimal VibrationUser — only used by VibrationData benchmarks that don't
// call into World (on_receive_vibration is a no-op).
// ---------------------------------------------------------------------------
struct BenchUser;

impl VibrationUser for BenchUser {
    fn get_listener_radius(&self) -> i32 {
        8
    }

    fn can_receive_vibration(
        &self,
        _world: &Arc<World>,
        _listener_pos: &BlockPos,
        _event: &GameEvent,
        _context: &GameEventContext,
    ) -> bool {
        true
    }

    fn on_receive_vibration<'a>(
        &'a self,
        _world: &'a Arc<World>,
        _listener_pos: &'a BlockPos,
        _event: &'a GameEvent,
        _context: &'a GameEventContext,
        _receiving_distance: f32,
        _source_entity: Option<&'a Arc<dyn EntityBase>>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

// ---------------------------------------------------------------------------
// VibrationSelector benchmarks
// ---------------------------------------------------------------------------

fn bench_vibration_selector(c: &mut Criterion) {
    let mut group = c.benchmark_group("vibration_selector");

    group.bench_function("add_candidate_and_choose", |b| {
        b.iter_batched(
            || {
                let selector = VibrationSelector::new();
                let info = VibrationInfo {
                    game_event: GameEvent::Step,
                    pos: Vector3::new(0.0, 0.0, 0.0),
                    source_entity: None,
                    distance: 5.0,
                    tick: 100,
                };
                (selector, info)
            },
            |(mut selector, info)| {
                selector.add_candidate(info);
                black_box(selector.choose(101));
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("add_10_candidates_choose_best", |b| {
        b.iter_batched(
            || {
                let mut selector = VibrationSelector::new();
                for i in 0..10 {
                    let info = VibrationInfo {
                        game_event: GameEvent::Step,
                        pos: Vector3::new(i as f64, 0.0, 0.0),
                        source_entity: None,
                        distance: 10.0 - i as f32,
                        tick: 100,
                    };
                    selector.add_candidate(info);
                }
                selector
            },
            |mut selector| {
                black_box(selector.choose(101));
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// VibrationData tick cycle benchmarks
// ---------------------------------------------------------------------------

fn bench_vibration_data_tick(c: &mut Criterion) {
    let mut group = c.benchmark_group("vibration_data");

    group.bench_function("tick_with_vibration", |b| {
        b.iter_batched(
            || {
                let mut data = VibrationData::new();
                let info = VibrationInfo {
                    game_event: GameEvent::Step,
                    pos: Vector3::new(0.0, 0.0, 0.0),
                    source_entity: None,
                    distance: 5.0,
                    tick: 0,
                };
                data.selector_mut().add_candidate(info);
                let user = BenchUser;
                data.try_select_and_schedule(1, &user);
                data
            },
            |mut data| {
                black_box(data.tick_receive());
                black_box(data.consume_current());
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("tick_no_vibration", |b| {
        b.iter_batched(
            VibrationData::new,
            |mut data| {
                black_box(data.tick_receive());
                black_box(data.consume_current());
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Redstone strength lookup benchmarks
// ---------------------------------------------------------------------------

fn bench_redstone_strength(c: &mut Criterion) {
    let mut group = c.benchmark_group("redstone_strength");

    for dist in [0.0, 4.0, 8.0, 16.0] {
        group.bench_function(format!("distance_{dist:.0}"), |b| {
            b.iter(|| black_box(get_redstone_strength_for_distance(dist, 8)));
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Atomic vs Mutex game_time read (microbenchmark)
// ---------------------------------------------------------------------------

fn bench_game_time_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("game_time_read");

    let atomic_time = AtomicI64::new(12345);
    let mutex_time = Mutex::new(12345i64);

    group.bench_function("atomic_load", |b| {
        b.iter(|| black_box(atomic_time.load(Ordering::Relaxed)));
    });

    group.bench_function("mutex_lock_and_read", |b| {
        b.iter(|| black_box(*mutex_time.lock().unwrap()));
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// AtomicI32 vs Mutex<i32> for last_vibration_frequency
// ---------------------------------------------------------------------------

fn bench_vibration_frequency_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("vibration_frequency_write");

    let atomic_freq = AtomicI32::new(0);
    let mutex_freq = Mutex::new(0i32);

    group.bench_function("atomic_store", |b| {
        b.iter(|| {
            atomic_freq.store(5, Ordering::Relaxed);
            black_box(());
        });
    });

    group.bench_function("mutex_lock_and_write", |b| {
        b.iter(|| {
            let mut val = mutex_freq.lock().unwrap();
            *val = 5;
            black_box(*val);
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// DashMap sensor index lookup (simulates game_event dispatch)
// ---------------------------------------------------------------------------

fn bench_sensor_index_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("sensor_index_lookup");

    let sensors: DashMap<Vector2<i32>, FxHashSet<BlockPos>> = DashMap::new();
    for x in -8..=8i32 {
        for z in -8..=8i32 {
            let mut set = FxHashSet::default();
            for i in 0..5 {
                set.insert(BlockPos(Vector3::new(x * 16 + i, 64, z * 16 + i)));
            }
            sensors.insert(Vector2::new(x, z), set);
        }
    }

    group.bench_function("iterate_all_289_chunks", |b| {
        b.iter(|| {
            for x in -8..=8i32 {
                for z in -8..=8i32 {
                    if let Some(chunk_sensors) = sensors.get(&Vector2::new(x, z)) {
                        for pos in chunk_sensors.iter() {
                            black_box(pos);
                        }
                    }
                }
            }
        });
    });

    group.bench_function("single_chunk_lookup", |b| {
        b.iter(|| {
            if let Some(chunk_sensors) = sensors.get(&Vector2::new(0, 0)) {
                for pos in chunk_sensors.iter() {
                    black_box(pos);
                }
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_vibration_selector,
    bench_vibration_data_tick,
    bench_redstone_strength,
    bench_game_time_read,
    bench_vibration_frequency_write,
    bench_sensor_index_lookup,
);
criterion_main!(benches);
