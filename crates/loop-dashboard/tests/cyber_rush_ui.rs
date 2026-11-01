//! Cyber Rush Phase 2 integration checks: the arcade physics contract is
//! present in the served asset — single physics config block, Space=nitro /
//! Shift=drift separation, drift skill gates, collision cooldown + knockback,
//! AI lane logic, and full state reset on restart.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use loop_dashboard::api::{build_app, AppOptions};

async fn app() -> axum::Router {
    let (router, _monitor) = build_app(AppOptions {
        demo: false,
        db: None,
        sys_demo: true,
        sys_interval: std::time::Duration::from_millis(2000),
        console_state: None,
        auth_store: None,
    })
    .unwrap();
    router
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let code = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (code, String::from_utf8_lossy(&bytes).into_owned())
}

fn js() -> String {
    include_str!("../src/assets/console-app.js").to_string()
}

#[tokio::test]
async fn cyber_rush_asset_is_served_as_javascript() {
    let app = app().await;
    let (code, body) = get(&app, "/console/console-app.js").await;
    assert_eq!(code, StatusCode::OK);
    assert!(body.contains("function crStep"), "physics step must exist");
    assert!(body.contains("requestAnimationFrame"), "must use RAF");
}

/// Every tunable lives in ONE config object, not scattered in render code.
#[test]
fn physics_config_is_centralised() {
    let src = js();
    // The config block itself.
    assert!(src.contains("const CRP = {"), "CRP config block must exist");
    for k in [
        "MAX_SPEED",
        "ACCELERATION",
        "BRAKE_FORCE",
        "STEERING_RESPONSE",
        "LATERAL_FRICTION",
        "DRIFT_GRIP",
        "NITRO_MULTIPLIER",
        "NITRO_CAPACITY",
        "NITRO_REGEN_FAST",
        "COLLISION_PENALTY_HEAD",
        "COLLISION_GRACE",
        "PLAYER_WIDTH",
    ] {
        assert!(src.contains(&format!("  {k}:"),), "missing config key {k}");
    }
    // Physics reads from it.
    assert!(src.contains("const P = CRP;"), "crStep must read CRP");
    // And no magic numbers linger where they were removed.
    assert!(
        !src.contains("g.speed - 11200 * dt"),
        "brake force must come from config"
    );
    assert!(
        !src.contains("* 1.32,",),
        "nitro multiplier must come from config"
    );
}

/// Space (and N) fire nitro; Shift fires drift — never the same key.
#[test]
fn controls_split_nitro_and_drift() {
    let src = js();
    assert!(
        src.contains("K[' '] || K['n'] || K['N'] || T.nitro"),
        "Space/N must be nitro"
    );
    assert!(src.contains("K['Shift'] || T.drift"), "Shift must be drift");
    // The old binding (Shift as nitro) must be gone.
    assert!(!src.contains("K['Shift'] || K['n'] || K['N'] || T.nitro"));
    // Touch pad for drift exists and is bound.
    assert!(
        src.contains(r#"id="crDrift""#),
        "touch drift pad must exist"
    );
    assert!(
        src.contains("bindHold('crDrift','drift')"),
        "drift pad must bind"
    );
    // Help text teaches the new scheme.
    assert!(src.contains("Shift drift"), "help must mention Shift drift");
    assert!(!src.contains("Shift/N nitro"), "stale help must be gone");
}

/// Drift is an active mechanic with skill gates, not a free score faucet.
#[test]
fn drift_requires_speed_steering_and_banks_on_release() {
    let src = js();
    assert!(
        src.contains("wantDrift && vRel2 > P.DRIFT_SPEED_MIN"),
        "drift needs speed"
    );
    assert!(
        src.contains("Math.abs(g.steer) > P.DRIFT_HOLD_STEER"),
        "drift needs steering input"
    );
    // Score accrual is speed-gated (no stationary farming).
    assert!(
        src.contains("if (vRel2 > P.DRIFT_SPEED_MIN) {\n      g.driftScore"),
        "drift score must be speed-gated"
    );
    // Banking happens only when the drift ends cleanly.
    assert!(
        src.contains("wasDrifting && g.driftT > P.DRIFT_MIN_TIME"),
        "banking needs minimum duration"
    );
    // Visuals tied to the state.
    assert!(
        src.contains("if (g.drifting && !g.reducedMotion) crEmitSmoke(dt)"),
        "smoke must emit while drifting"
    );
    assert!(src.contains("crAddSkid()"), "skid marks must be laid");
}

/// Nitro cannot run with an empty meter, drains while held, regenerates.
#[test]
fn nitro_is_metered() {
    let src = js();
    assert!(
        src.contains("g.nitro > P.NITRO_MIN_ENERGY && wantNitro"),
        "nitro needs energy"
    );
    assert!(src.contains("dt / P.NITRO_CAPACITY"), "nitro must drain");
    assert!(
        src.contains("P.NITRO_REGEN_FAST : P.NITRO_REGEN_SLOW"),
        "nitro must regenerate"
    );
    // Boost adds camera intensity without bypassing boundaries.
    assert!(
        src.contains("P.NITRO_SHAKE"),
        "nitro shake must be configured"
    );
}

/// One contact = one penalty; knockback separates the cars.
#[test]
fn collision_has_cooldown_and_knockback() {
    let src = js();
    assert!(
        src.contains("if (g.crashGrace > 0) return;"),
        "crCrash must respect the grace window"
    );
    assert!(
        src.contains("g.crashGrace = Math.max(g.crashGrace, P.COLLISION_GRACE_HOLD)"),
        "scrapes must refresh, not re-penalise"
    );
    assert!(
        src.contains("g.crashGrace = P.COLLISION_GRACE"),
        "fresh hit sets the cooldown"
    );
    assert!(src.contains("car.bumped = 1.2"), "AI must react to hits");
    assert!(src.contains("crSparks("), "collision sparks must fire");
    assert!(
        src.contains("g.vlat -= dir * shove"),
        "player must be shoved"
    );
}

/// AI traffic drives like individuals: lanes, overtakes, reactions.
#[test]
fn ai_uses_lanes_and_reacts() {
    let src = js();
    assert!(src.contains("targetLane"), "AI must change lanes");
    assert!(src.contains("aggression"), "AI must have personality");
    assert!(src.contains("car.bumped > 0"), "AI must react after a bump");
    assert!(
        src.contains("laneCentre(car.targetLane)"),
        "AI must steer toward lane centres"
    );
}

/// Restart clears every piece of physics state — no stale listeners/inputs.
#[test]
fn restart_resets_input_and_effects() {
    let src = js();
    assert!(
        src.contains("CRG.keys = Object.create(null);"),
        "keys must clear"
    );
    assert!(src.contains("drift:false"), "touch drift flag must reset");
    assert_eq!(
        src.matches("CRG.touch = { left:false, right:false, gas:false, brake:false, nitro:false, drift:false };").count(),
        3,
        "restart, blur and teardown must all reset touch state"
    );
}

/// The game keys never scroll the page while playing.
#[test]
fn game_keys_prevent_scroll() {
    let src = js();
    assert!(
        src.contains("e.key === ' ' || e.key.startsWith('Arrow')"),
        "space/arrows must preventDefault"
    );
}

/* =====================================================================
PHASE 3 — cinematic effects contract
===================================================================== */

/// One pooled FX engine with hard caps backs every new effect class.
#[test]
fn phase3_particle_engine_is_pooled_and_capped() {
    let src = js();
    assert!(src.contains("const CR_FX = {"), "FX engine must exist");
    assert!(
        src.contains("caps: { exhaust: 70, dust: 40 }"),
        "pool caps must be declared up front"
    );
    assert!(
        src.contains("function crFxSpawn(pool, p)"),
        "spawner must exist"
    );
    assert!(
        src.contains("function crFxStep(arr, dt, drag, grav)"),
        "stepper must exist"
    );
    // The stepper compacts in place — no splice/alloc in the hot loop.
    assert!(
        src.contains("arr[w++] = p;"),
        "must compact with a write index"
    );
    assert!(src.contains("arr.length = w;"), "must truncate in place");
    // Recycle path prevents unbounded growth when the cap is hit.
    assert!(
        src.contains("if (minLife < p.life) { arr[oldest] = p; return p; }"),
        "cap-hit path must recycle the deadest particle"
    );
}

/// Every nitro visual eases through one scalar — no hard on/off cuts.
#[test]
fn phase3_nitro_transitions_are_eased() {
    let src = js();
    assert!(src.contains("nitroVis: 0,"), "state must carry nitroVis");
    assert!(
        src.contains("((g.nitroActive ? 1 : 0) - g.nitroVis)"),
        "nitroVis must ease toward the active flag"
    );
    // All four nitro-driven visuals read the eased value.
    assert!(
        src.contains("+ g.nitroVis * P.FOV_NITRO_GAIN"),
        "FOV must ease"
    );
    assert!(
        src.contains("(0.26 + g.nitroVis * 0.16)"),
        "vignette must ease"
    );
    // BUGFIX 8: the horizon glow rect became an atmospheric gradient;
    // its upper-haze stop still eases with nitroVis.
    assert!(src.contains("(0.34 + nv * 0.10)"), "horizon haze must ease");
    assert!(src.contains("CRG.nitroVis * 0.35"), "road neon must ease");
}

/// Drift visuals scale with intensity and stay physics-scored.
#[test]
fn phase3_drift_fx_scale_with_intensity() {
    let src = js();
    // Smoke rate scales with slip beyond the drift threshold.
    assert!(
        src.contains("(slip - P.DRIFT_SLIP_MIN) / P.DRIFT_SLIP_MIN"),
        "intensity must derive from slip vs threshold"
    );
    assert!(src.contains("34 + 46 * inten"), "smoke rate must scale");
    // Directional smoke carries vx into the slide.
    assert!(
        src.contains("dirX * (10 + Math.random() * 26)"),
        "smoke must sweep directionally"
    );
    // Neon tyre sparks reuse the spark pool with a tint flag.
    assert!(src.contains("neon: true,"), "drift embers must be tinted");
    // Scoring stays physics-side: no score mutation inside the smoke emitter.
    let smoke_start = src.find("function crEmitSmoke").expect("emitter");
    let smoke_end = src[smoke_start..]
        .find("\n}\n")
        .map(|i| smoke_start + i)
        .unwrap_or(src.len());
    let smoke_body = &src[smoke_start..smoke_end];
    assert!(!smoke_body.contains("g.score"), "FX must never award score");
}

/// Collision feedback scales with impact severity; penalties unchanged.
#[test]
fn phase3_collision_fx_scale_but_penalties_do_not() {
    let src = js();
    assert!(
        src.contains("const impactK = Math.min(1, preSpeed / g.maxSpeed);"),
        "severity must come from closing speed"
    );
    assert!(
        src.contains("0.30 + impactK * 0.34"),
        "head-on shake must scale"
    );
    assert!(
        src.contains("0.16 + impactK * 0.22"),
        "glance shake must scale"
    );
    // Penalties still come from config only.
    assert!(src.contains("P.COLLISION_PENALTY_HEAD"));
    assert!(src.contains("P.COLLISION_SPEED_HEAD"));
    // Slow-motion is sim-only and configured.
    assert!(
        src.contains("COLLISION_SLOWMO_T:"),
        "slow-mo duration must be config"
    );
    assert!(
        src.contains("simDt = dt * (CRP.COLLISION_SLOWMO_K"),
        "slow-mo must scale only the simulation clock"
    );
}

/// Camera gains drift lean, suspension bob and post-hit stabilization.
#[test]
fn phase3_camera_refinements() {
    let src = js();
    assert!(
        src.contains("CAM_LAG_STAB:"),
        "stabilization lag must be config"
    );
    assert!(
        src.contains("CAM_DRIFT_K:"),
        "drift follow factor must be config"
    );
    assert!(src.contains("g.camBob"), "suspension bob must exist");
    assert!(
        src.contains("(CRG.camY || 0) + (CRG.camBob || 0)"),
        "projection must include the bob"
    );
}

/// Pause freezes screen-space FX; restart/teardown clear every pool.
#[test]
fn phase3_lifecycle_freezes_and_clears() {
    let src = js();
    assert!(
        src.contains("g._dtDraw = g.state === 'playing' ? dt : 0;"),
        "FX clock must stop when not playing"
    );
    for site in ["crFxClear();                                     // Phase 3 pools"] {
        assert_eq!(
            src.matches(site).count(),
            2,
            "restart AND teardown must both call crFxClear"
        );
    }
}

/// Speed lines are deterministic (no per-frame random flicker).
#[test]
fn phase3_speed_lines_are_stable() {
    let src = js();
    assert!(
        src.contains("const h1 = crHash(i * 7 + 3)"),
        "line anchors must be hash-seeded"
    );
}

/* =====================================================================
PHASE 6 - complete arcade experience contract
===================================================================== */

/// Three modes with distinct rules; difficulty presets scale real values.
#[test]
fn phase6_modes_and_difficulty_are_configured() {
    let src = js();
    assert!(src.contains("const CR_MODES = {"), "mode table must exist");
    assert!(src.contains("label:'Quick Race'"), "quick race mode");
    assert!(src.contains("label:'Time Attack'"), "time attack mode");
    assert!(src.contains("label:'Endless'"), "endless mode");
    assert!(
        src.contains("const CR_DIFFS = {"),
        "difficulty table must exist"
    );
    for k in ["traffic:6", "traffic:9", "traffic:12"] {
        assert!(src.contains(k), "difficulty traffic scaling missing {k}");
    }
    assert!(
        src.contains("oppSpeed:0.78") && src.contains("oppSpeed:1.24"),
        "opponent speed must scale by difficulty"
    );
    assert!(
        src.contains("penaltyK:0.55") && src.contains("penaltyK:1.35"),
        "collision forgiveness must scale by difficulty"
    );
    assert!(
        src.contains("let oppK = D.oppSpeed, aggK = D.aggression;"),
        "AI must consume difficulty"
    );
    assert!(
        src.contains("* D.penaltyK"),
        "collisions must consume difficulty"
    );
}

/// Five garage cars with distinct stats; physics scales from them.
#[test]
fn phase6_garage_has_five_distinct_cars() {
    let src = js();
    assert!(src.contains("const CR_CARS = ["), "car roster must exist");
    for id in ["aero", "vortex", "kaida", "brute", "phantom"] {
        assert!(src.contains(&format!("id:'{id}',")), "missing car {id}");
    }
    assert!(
        src.contains("function crCarSpec()"),
        "spec resolver must exist"
    );
    assert!(
        src.contains("CRP.ACCELERATION * car.accel"),
        "acceleration must scale per car"
    );
    assert!(
        src.contains("CRP.MAX_SPEED * car.top"),
        "top speed must scale per car"
    );
    assert!(
        src.contains("const carP = crCarSpec().P;"),
        "step must use car spec"
    );
}

/// Customization renders: paint gradient, neon underglow, wheels, plate.
#[test]
fn phase6_customization_renders() {
    let src = js();
    assert!(src.contains("function crShade("), "paint shading helper");
    assert!(
        src.contains("CR_SAVE.custom.body"),
        "body paint must be read"
    );
    assert!(
        src.contains("CR_SAVE.custom.neon"),
        "neon accent must be read"
    );
    assert!(
        src.contains("CR_SAVE.custom.wheel"),
        "wheel style must be read"
    );
    assert!(
        src.contains("CR_SAVE.custom.spoiler"),
        "spoiler style must be read"
    );
    assert!(
        src.contains("CR_SAVE.custom.plate"),
        "plate text must be read"
    );
    assert!(
        src.contains("spId === 'duck'") && src.contains("spId === 'twin'"),
        "spoiler styles must differ visually"
    );
}

/// Persistence: one key, validated load, reset path.
#[test]
fn phase6_persistence_is_validated() {
    let src = js();
    assert!(
        src.contains("const CR_SAVE_KEY = 'cyber-rush-save-v1';"),
        "single storage key"
    );
    assert!(
        src.contains("function crSaveValidate("),
        "load-time validation"
    );
    assert!(
        src.contains("localStorage.removeItem(CR_SAVE_KEY)"),
        "records reset path"
    );
    assert!(
        src.contains("num(raw.records, k, def.records[k])"),
        "records need record fallbacks"
    );
    assert!(
        src.contains("num(raw.totals, k, def.totals[k])"),
        "totals need totals fallbacks"
    );
}

/// Achievements test real stats and cannot double-award.
#[test]
fn phase6_achievements_dedupe_and_use_stats() {
    let src = js();
    assert!(
        src.contains("const CR_ACHIEVEMENTS = ["),
        "achievement list"
    );
    assert!(
        src.contains("if (CR_SAVE.achievements.includes(a.id)) continue;"),
        "duplicate awards must be blocked"
    );
    assert!(
        src.contains("r.maxKmh>=300"),
        "speed achievement uses speed"
    );
    assert!(
        src.contains("r.crashes===0"),
        "clean achievement uses crashes"
    );
    assert!(
        src.contains("s.totals.distanceKm>=50"),
        "distance uses totals"
    );
}

/// Championship: points from results only, progress saved, restartable.
#[test]
fn phase6_championship_points_and_progress() {
    let src = js();
    assert!(src.contains("const CR_CHAMP_EVENTS = ["), "event schedule");
    assert!(
        src.contains("function crChampPoints(result)"),
        "points derive from race results"
    );
    assert!(src.contains("ch.eventIdx++;"), "events advance");
    assert!(
        src.contains("CR_SAVE.champ = null;"),
        "championship completes"
    );
    assert!(
        src.contains("crChampNew(); crSaveWrite();"),
        "championship restart persists"
    );
}

/// Audio: gear blip, tyre squeal loop, rpm-aware engine - all behind CRA.on.
#[test]
fn phase6_audio_additions_respect_toggle() {
    let src = js();
    assert!(src.contains("function crSfxGear()"), "gear shift sound");
    assert!(src.contains("function crDriftAudio("), "drift tyre sound");
    assert!(
        src.contains("crEngineUpdate(g.nitroActive ? 1 : speedRatio, g.rpm, g.gear)"),
        "engine note must follow rpm/gear"
    );
    let gear = src
        .split("function crSfxGear")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap();
    assert!(gear.contains("CRA.on"), "gear sfx must respect toggle");
}

/// Game flow: menu screens exist and launch wires mode+difficulty.
#[test]
fn phase6_flow_menu_to_race() {
    let src = js();
    for id in [
        "crMenuScreen",
        "crDiffScreen",
        "crGarageScreen",
        "crRecordsScreen",
        "crChampScreen",
        "crPlayScreen",
    ] {
        assert!(src.contains(&format!("id=\"{id}\"")), "missing screen {id}");
    }
    assert!(
        src.contains("function crLaunchRace(mode, diff, champRun)"),
        "race launcher must take mode+difficulty"
    );
    assert!(src.contains("crScreen('crMenuScreen')"), "menu router");
}

/* =====================================================================
TASK 7.1 · LAP & CHECKPOINT SYSTEM — static contract checks.
The behavioural verification (ordering, skip/duplicate/reverse
rejection, lap timing, mode compatibility) lives in the browser suite;
these assert the architecture is present and wired.
===================================================================== */

/// Checkpoints are defined on real track geometry (segment indices), not
/// timers or distance thresholds.
#[test]
fn checkpoints_are_world_geometry_not_timers() {
    let src = js();
    assert!(
        src.contains("function crMakeCheckpoints"),
        "checkpoint builder must exist"
    );
    assert!(
        src.contains("const CR_CHECKPOINTS"),
        "checkpoint count constant must exist"
    );
    // Gates are segment indices on the loop...
    assert!(
        src.contains("Math.round(i * total / count) % total"),
        "gates must be evenly spaced segment indices on the loop"
    );
    // ...and crossing detection compares wrapped segment deltas.
    assert!(
        src.contains("function crWrapSeg"),
        "wrapped-delta helper must exist"
    );
    assert!(
        !src.contains("cpTimer"),
        "no timer-based checkpoints allowed"
    );
}

/// The state machine enforces order, rejects duplicates and reverses.
#[test]
fn checkpoint_state_machine_enforces_order() {
    let src = js();
    assert!(
        src.contains("function crCheckpointStep"),
        "checkpoint step must exist"
    );
    // Order: only the expected gate can activate.
    assert!(
        src.contains("const cp = g.cps[L.nextCp]"),
        "only the next expected gate may activate"
    );
    // Reverse rejected: forward motion required.
    assert!(
        src.contains("if (moved <= 0) return null"),
        "reverse/stationary ticks must be rejected"
    );
    // Duplicates impossible: hit flag checked.
    assert!(
        src.contains("if (!cp || cp.hit) return null"),
        "already-hit gates must not re-activate"
    );
    // Skips rejected: crossing must occur within this tick's arc.
    assert!(
        src.contains("if (rel <= 0 || rel > moved) return null"),
        "gates outside the tick's travel arc must not activate"
    );
}

/// Lap progression: current/total laps, final-lap notice, finish freeze.
#[test]
fn lap_progression_and_finish_wiring() {
    let src = js();
    for needle in [
        "totalLaps",             // configured lap count
        "L.lap >= L.totalLaps",  // final-lap completion branch
        "'FINAL LAP'",           // final lap announcement
        "'FINISH!'",             // finish notification
        "L.finished = true",     // finish freezes progression
        "return crFinish(true)", // finish hands off to the results flow
    ] {
        assert!(src.contains(needle), "lap progression missing: {needle}");
    }
    // Default configuration is 3 laps.
    assert!(
        src.contains("laps:3") || src.contains("laps: 3"),
        "default lap count must be 3"
    );
}

/// Timing: starts at GO, per-lap splits, best lap, pause-freeze via the
/// simulation clock.
#[test]
fn lap_timing_architecture() {
    let src = js();
    for needle in [
        "g.raceTimeMs = 0;                    // timer starts NOW",
        "lapTimes.push(lapMs)",        // splits recorded
        "L.bestLapMs = lapMs",         // best lap tracked
        "prevLapMs",                   // previous lap exposed
        "crFmtTime(CR_LAP.bestLapMs)", // best lap shown in HUD
    ] {
        assert!(src.contains(needle), "lap timing missing: {needle}");
    }
}

/// Mode compatibility: quick/champ are lap races; timeattack keeps its
/// clock; endless has no artificial finish.
#[test]
fn mode_compatibility_rules() {
    let src = js();
    // Quick is lap-based...
    assert!(
        src.contains("cps: (mode === 'quick')"),
        "checkpoints exist only for lap-based modes"
    );
    // ...and no longer ends on a score target.
    assert!(
        !src.contains("g.mode === 'quick' && g.score >= g.targetScore"),
        "quick mode must not end on a score target"
    );
    // Endless never gets gates or a lap finish.
    assert!(
        src.contains("mode === 'endless' ? 1 + Math.min"),
        "endless survival scoring must remain"
    );
    // Time Attack still owns the countdown clock.
    assert!(
        src.contains("g.timeLeftMs -= dt * 1000"),
        "time attack clock must keep running"
    );
}

/// Results screen reports lap telemetry; persistence stores best lap.
#[test]
fn results_and_persistence_lap_data() {
    let src = js();
    for needle in [
        "raceWonByLaps", // result flag
        "crRsLaps",      // results grid cell
        "bestLapMs: 0,                        // TASK 7.1",
        "result.bestLapMs < (r.bestLapMs || Infinity)",
    ] {
        assert!(src.contains(needle), "lap data wiring missing: {needle}");
    }
}
