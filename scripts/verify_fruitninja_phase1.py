#!/usr/bin/env python3
"""PHASE 1 QA: time-based gameplay speed & difficulty balancing — Fruit Ninja ONLY.

Verifies in real Chromium (http://127.0.0.1:8912):
  1. JS parses cleanly (no page errors on load or during play).
  2. The elapsed difficulty clock exists, advances only while playing,
     and freezes while paused.
  3. Wave-gap curve now DECREASES with elapsed time (the old Arcade curve
     was inverted — fastest waves at second zero).
  4. fnDiff() rises smoothly 0→1 and never exceeds 1.
  5. Launch-speed boost grows with difficulty but the apex stays on-screen.
  6. Wave size stays within the original per-mode maximum (Classic ≤4, Arcade ≤5).
  7. Regression: scoring, lives, miss/bomb penalties, game-over logic intact.

Order matters: the live-run regression plays FIRST, before any direct
fnSpawnWave() probing (whose staggered setTimeout spawns must never leak
into a real run).
"""
import json, sys, time
from playwright.sync_api import sync_playwright

BASE = "http://127.0.0.1:8912"
OUT = "/tmp/fn-shots/phase1"
RESULTS = []

def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  [{detail}]" if detail else ""))

with sync_playwright() as p:
    b = p.chromium.launch()
    page = b.new_page(viewport={"width": 1366, "height": 768})
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
    page.goto(BASE + "/console"); page.wait_for_selector("#viewRoot")
    page.evaluate("location.hash = '#funtime/fn'")
    page.wait_for_selector("#fnCanvas"); page.wait_for_timeout(600)

    check("load/noPageErrors", not errors, "; ".join(errors[:3]))

    # ---------- plumbing present ----------
    has = page.evaluate("""(() => ({
        elapsed: 'elapsedMs' in window.FNG,
        diffFn: typeof window.fnDiff === 'function',
        gapFn: typeof window.fnWaveGapMs === 'function',
        spanConst: typeof FN_DIFF_CLASSIC_MS !== 'undefined',
    }))()""")
    check("p1/clockAndCurveExposed", all(has.values()), json.dumps(has))

    # ---------- LIVE RUN FIRST: clock + regression ----------
    page.click(".fn-mode[data-mode='classic']")
    page.wait_for_timeout(2600)  # grace (1200ms) + a few waves
    st = page.evaluate("({state: window.FNG.state, elapsed: window.FNG.elapsedMs,"
                       " waves: window.FNG.waveCount, score: window.FNG.score})")
    check("run/elapsedAdvancesWhilePlaying",
          st["state"] == "playing" and 1300 < st["elapsed"] <= 3600,
          json.dumps(st))

    # pause freezes the difficulty clock (like the arcade countdown)
    page.keyboard.press("p"); page.wait_for_timeout(200)
    frozen = page.evaluate("window.FNG.elapsedMs")
    page.wait_for_timeout(900)
    frozen2 = page.evaluate("({e: window.FNG.elapsedMs, s: window.FNG.state})")
    check("run/pauseFreezesDifficultyClock",
          frozen2["s"] == "paused" and frozen2["e"] == frozen,
          f"{frozen} -> {frozen2['e']} ({frozen2['s']})")
    page.screenshot(path=f"{OUT}/01-paused.png")

    page.keyboard.press("p"); page.wait_for_timeout(500)  # resume
    resumed = page.evaluate("window.FNG.elapsedMs")
    check("run/resumeContinuesClock", resumed > frozen, f"{frozen} -> {resumed}")

    # slicing still scores (regression): synthetic fast pointer swipe
    scored = page.evaluate("""(() => new Promise(res => {
        const FNG = window.FNG;
        const before = FNG.score;
        const c = document.getElementById('fnCanvas');
        const r = c.getBoundingClientRect();
        let tries = 0;
        const timer = setInterval(() => {
            if (FNG.fruits.length) {
                const f = FNG.fruits.find(x => !x.bomb);
                if (f) {
                    const scale = FN_WORLD_W / FNG.w;
                    const x0 = f.x / scale + r.left, y0 = f.y / scale + r.top;
                    const ev = (t, x, y) => new PointerEvent(t, {
                        bubbles: true, clientX: x, clientY: y, pointerId: 1});
                    c.dispatchEvent(ev('pointerdown', x0 - 120, y0 + 40));
                    for (let k = 1; k <= 14; k++)
                        c.dispatchEvent(ev('pointermove', x0 - 120 + k * 17, y0 + 40 - k * 6));
                    c.dispatchEvent(new PointerEvent('pointerup', {bubbles: true, pointerId: 1}));
                }
            }
            if (FNG.score > before || ++tries > 60) {
                clearInterval(timer); res(FNG.score - before);
            }
        }, 120);
    }))()""")
    check("run/slicingStillScores", scored > 0, f"+{scored} pts")

    # misses still cost a life in classic (regression); game over follows
    over = page.evaluate("""(() => new Promise(res => {
        const FNG = window.FNG;
        const l0 = FNG.lives;
        const dl = Date.now() + 60000;
        const iv = setInterval(() => {
            if (FNG.state === 'over' || Date.now() > dl) {
                clearInterval(iv);
                res({l0, l1: FNG.lives, state: FNG.state,
                     title: (document.getElementById('fnOvTitle') || {}).textContent});
            }
        }, 150);
    }))()""")
    check("run/missStillCostsLifeAndGameOver",
          over["l1"] < over["l0"] and over["state"] == "over"
          and over["title"] == "Game Over", json.dumps(over))
    page.screenshot(path=f"{OUT}/02-gameover.png")

    # restart resets the difficulty clock and the run
    page.click("#fnRestartBtn"); page.wait_for_timeout(300)
    rst = page.evaluate("({state: window.FNG.state, e: window.FNG.elapsedMs,"
                        " s: window.FNG.score, l: window.FNG.lives, w: window.FNG.waveCount})")
    check("run/restartResetsClock",
          rst["state"] == "playing" and rst["e"] < 400 and rst["s"] == 0
          and rst["l"] == 3 and rst["w"] == 0, json.dumps(rst))

    # leave the run cleanly before pure-function probing
    page.evaluate("location.hash = '#funtime'")
    page.wait_for_timeout(400)
    page.evaluate("location.hash = '#funtime/fn'")
    page.wait_for_selector("#fnCanvas"); page.wait_for_timeout(400)

    # ---------- PURE FUNCTION PROBES (no live run active) ----------
    curve = page.evaluate("""(() => {
        const out = [];
        for (let ms of [0, 5000, 10000, 20000, 40000, 60000, 80000, 120000]) {
            window.FNG.elapsedMs = ms;
            out.push([ms, window.fnDiff()]);
        }
        return out;
    })()""")
    vals = [v for _, v in curve]
    mono = all(y >= x - 1e-9 for x, y in zip(vals, vals[1:]))
    check("p1/diffMonotonicAndClamped", mono and vals[0] == 0 and vals[-1] == 1.0,
          json.dumps(curve))

    spans = page.evaluate("""(() => {
        const FNG = window.FNG;
        FNG.mode = 'classic'; FNG.elapsedMs = 40000;
        const c40 = window.fnDiff();
        FNG.mode = 'arcade';
        const a40 = window.fnDiff();
        return {c40, a40};
    })()""")
    check("p1/classicRampsSlowerThanArcade",
          0 < spans["c40"] < spans["a40"] <= 1, json.dumps(spans))

    gaps = page.evaluate("""(() => {
        const FNG = window.FNG; const res = {};
        for (const mode of ['classic', 'arcade']) {
            FNG.mode = mode;
            const early = [], late = [];
            for (let i = 0; i < 60; i++) {
                FNG.elapsedMs = 1000;   early.push(window.fnWaveGapMs());
                FNG.elapsedMs = 55000;  late.push(window.fnWaveGapMs());
            }
            const avg = a => a.reduce((x, y) => x + y, 0) / a.length;
            res[mode] = {early: avg(early), late: avg(late)};
        }
        return res;
    })()""")
    for mode in ("classic", "arcade"):
        g = gaps[mode]
        check(f"p1/gapShrinksOverTime/{mode}", g["late"] < g["early"],
              f"early~{g['early']:.0f}ms late~{g['late']:.0f}ms")

    floors = page.evaluate("""(() => {
        const FNG = window.FNG; const res = {};
        for (const mode of ['classic', 'arcade']) {
            FNG.mode = mode; FNG.elapsedMs = 10 * 60000;
            let mn = 1e9;
            for (let i = 0; i < 200; i++) mn = Math.min(mn, window.fnWaveGapMs());
            res[mode] = mn;
        }
        return res;
    })()""")
    check("p1/gapFloorRespected",
          floors["classic"] >= 450 * 0.86 - 1 and floors["arcade"] >= 380 * 0.86 - 1,
          json.dumps(floors))

    apex = page.evaluate("""(() => {
        const FNG = window.FNG; const res = {};
        for (const mode of ['classic', 'arcade']) {
            FNG.mode = mode; res[mode] = {};
            for (const label of ['early', 'late']) {
                FNG.elapsedMs = label === 'early' ? 0 : 90 * 60000;
                let s = 0;
                for (let i = 0; i < 80; i++) s += window.fnLaunchSpeed(620);
                res[mode][label] = s / 80;
            }
        }
        return res;
    })()""")
    for mode in ("classic", "arcade"):
        a = apex[mode]
        check(f"p1/launchSpeedGrows/{mode}", a["late"] > a["early"],
              f"early~{a['early']:.0f} late~{a['late']:.0f}")
    capok = page.evaluate("""(() => {
        const FNG = window.FNG; const h = 620;
        FNG.mode = 'arcade'; FNG.elapsedMs = 10 * 60000;
        let mx = 0;
        for (let i = 0; i < 300; i++) {
            const v = window.fnLaunchSpeed(h);
            mx = Math.max(mx, (v * v) / (2 * 1500));
        }
        return {mx, limit: h * 0.96};
    })()""")
    check("p1/apexCappedOnScreen", capok["mx"] <= capok["limit"] + 1e-6,
          json.dumps(capok))

    # Wave size: capture the staggered setTimeout spawns SYNCHRONOUSLY so
    # nothing leaks into a live run. Bound must match the original maxima
    # (classic ≤ 4, arcade ≤ 5) and early waves must stay smaller.
    sizes = page.evaluate("""(() => {
        const FNG = window.FNG; const res = {};
        const realSetTimeout = window.setTimeout;
        for (const mode of ['classic', 'arcade']) {
            for (const phase of ['early', 'late']) {
                FNG.mode = mode;
                FNG.elapsedMs = phase === 'early' ? 0 : 30 * 60000;
                const perWave = [];
                for (let w = 0; w < 300; w++) {
                    let count = 0;
                    const cbs = [];
                    window.setTimeout = cb => { cbs.push(cb); return 0; };
                    try { window.fnSpawnWave(); } finally { window.setTimeout = realSetTimeout; }
                    // state is 'menu' here; the guard makes each cb a no-op,
                    // so count entities straight off the schedule instead.
                    count = cbs.length;
                    perWave.push(count);
                }
                res[mode + '_' + phase] = {
                    max: Math.max.apply(null, perWave),
                    avg: perWave.reduce((a, b) => a + b, 0) / perWave.length,
                };
            }
        }
        return res;
    })()""")
    check("p1/waveSizeBounded/classic", sizes["classic_late"]["max"] <= 4,
          json.dumps(sizes["classic_late"]))
    check("p1/waveSizeBounded/arcade", sizes["arcade_late"]["max"] <= 5,
          json.dumps(sizes["arcade_late"]))
    check("p1/earlyWavesSmaller/classic",
          sizes["classic_early"]["avg"] < sizes["classic_late"]["avg"],
          f"avg {sizes['classic_early']['avg']:.2f} -> {sizes['classic_late']['avg']:.2f}")
    check("p1/earlyWavesSmaller/arcade",
          sizes["arcade_early"]["avg"] < sizes["arcade_late"]["avg"],
          f"avg {sizes['arcade_early']['avg']:.2f} -> {sizes['arcade_late']['avg']:.2f}")

    # ---------- teardown cleanliness ----------
    page.evaluate("location.hash = '#funtime'")
    page.wait_for_timeout(500)
    clean = page.evaluate("""({
        canvas: !!document.getElementById('fnCanvas'),
        fsClass: document.body.className.includes('fn-fs'),
        playing: document.body.className.includes('ft-playing'),
    })""")
    check("teardown/noDomLeak", not clean["canvas"] and not clean["fsClass"]
          and not clean["playing"], json.dumps(clean))
    # NOTE: <style id="fn-style"> persists by design (id-guarded injection)

    check("final/noConsoleErrors", not errors, "; ".join(errors[:3]))

    b.close()

fails = [r for r in RESULTS if not r[1]]
print(f"\n{len(RESULTS) - len(fails)}/{len(RESULTS)} passed")
sys.exit(1 if fails else 0)
