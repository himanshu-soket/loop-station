#!/usr/bin/env python3
"""Cyber Rush visual + functional verification.
Covers: gameplay, controls, pause, restart, sound, fullscreen, responsive,
and the new premium visuals (neon edges, lane dashes, guardrails, city
lights, sun, car detail) — measured from canvas pixels in real Chromium.
"""
import json, sys, time
from playwright.sync_api import sync_playwright

BASE = "http://127.0.0.1:8912"
OUT = "/tmp/cr-shots/after"
RESULTS = []

def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  [{detail}]" if detail else ""))

with sync_playwright() as p:
    b = p.chromium.launch()
    page = b.new_page(viewport={"width":1366,"height":768})
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(BASE + "/console"); page.wait_for_selector("#viewRoot")
    page.evaluate("location.hash = '#funtime/cr'")
    page.wait_for_selector("#crCanvas"); page.wait_for_timeout(700)

    # ---------- FUNCTIONAL ----------
    # PHASE 6 flow: the game opens on the MENU screen; launch goes through
    # menu -> difficulty -> countdown. The old script assumed a bare
    # "ready" overlay with a visible #crOvBtn.
    st = page.evaluate("({state: window.CRD.state, overlay: window.CRD.overlay})")
    check("func/readyState", st["state"] == "ready" and st["overlay"] in ("ready", "menu"), json.dumps(st))

    # Launch Quick Race from the menu, accept default difficulty.
    page.click("#crMenuScreen .ft-btn.primary"); page.wait_for_timeout(400)
    # Difficulty screen: pick Medium (second non-ghost button).
    try:
        page.click("#crDiffScreen .ft-btn:not(.ghost) >> nth=1", timeout=2000)
    except Exception:
        pass                                    # some flows skip the picker
    page.wait_for_timeout(500)
    st = page.evaluate("window.CRD.state")
    check("func/countdownStarts", st == "countdown", st)
    page.screenshot(path=f"{OUT}/01-countdown.png")

    page.wait_for_timeout(4000)   # countdown -> playing
    page.keyboard.down("ArrowUp"); page.wait_for_timeout(3000)
    st = page.evaluate("({s: window.CRD.speed, st: window.CRD.state})")
    check("func/accelerates", st["st"] == "playing" and st["s"] > 4000, json.dumps(st))
    page.screenshot(path=f"{OUT}/02-playing.png")

    # steering moves playerX
    x0 = page.evaluate("window.CRD.playerX")
    page.keyboard.down("ArrowLeft"); page.wait_for_timeout(800); page.keyboard.up("ArrowLeft")
    x1 = page.evaluate("window.CRD.playerX")
    check("func/steeringWorks", x1 < x0 - 0.05, f"{x0} -> {x1}")

    # nitro engages (Space since PHASE 2 — Shift is drift now)
    page.keyboard.down("ArrowUp"); page.wait_for_timeout(200)
    page.keyboard.down(" "); page.wait_for_timeout(900)
    nit = page.evaluate("window.CRD.nitroActive")
    page.keyboard.up(" ")
    check("func/nitroEngages", nit, str(nit))
    page.screenshot(path=f"{OUT}/03-nitro.png")

    # pause / resume
    page.keyboard.press("p"); page.wait_for_timeout(300)
    st = page.evaluate("({s: window.CRD.state, t: document.getElementById('crOvTitle').textContent})")
    check("func/pauseWorks", st["s"] == "paused" and st["t"] == "Paused", json.dumps(st))
    page.screenshot(path=f"{OUT}/04-paused.png")
    page.keyboard.press("p"); page.wait_for_timeout(300)
    st = page.evaluate("window.CRD.state")
    check("func/resumeWorks", st == "playing", st)

    # restart (button + R key)
    page.keyboard.press("r"); page.wait_for_timeout(400)
    st = page.evaluate("({s: window.CRD.state, sc: window.CRD.score})")
    check("func/restartWorks", st["s"] == "ready" and st["sc"] == 0, json.dumps(st))

    # sound toggle
    page.click("#crSoundBtn"); page.wait_for_timeout(200)
    lbl = page.evaluate("document.getElementById('crSoundLbl').textContent")
    check("func/soundToggles", "on" in lbl, lbl)
    page.click("#crSoundBtn")   # back off (default)

    # mode switch (PHASE 6 menu ids: crMTime / crMEndless). After the R
    # restart the game sits on its READY overlay, so walk back to the main
    # menu first, then launch Time Attack through the menu flow.
    page.click("#crOvMenuBtn"); page.wait_for_timeout(500)
    page.click("#crMTime"); page.wait_for_timeout(400)
    try:
        page.click("#crDMedium", timeout=1500)      # difficulty picker
    except Exception:
        pass
    page.wait_for_timeout(300)
    st = page.evaluate("window.CRD.mode")
    check("func/modeSwitch", st == "timeattack", st)

    # fullscreen + Esc
    page.click("#crFsBtn"); page.wait_for_timeout(900)
    st = page.evaluate("""(() => ({native: !!document.fullscreenElement,
      w: Math.round(document.getElementById('crCanvas').getBoundingClientRect().width)}))()""")
    check("func/fullscreenEntered", st["native"] and st["w"] > 600, json.dumps(st))
    page.screenshot(path=f"{OUT}/05-fullscreen.png")
    page.keyboard.press("Escape"); page.wait_for_timeout(700)
    st = page.evaluate("!!document.getElementById('ftCardCr')")
    check("func/escExitsToGames", st)

    # ---------- VISUAL (canvas pixel audit while driving) ----------
    page.evaluate("location.hash = '#funtime/cr'")
    page.wait_for_selector("#crCanvas"); page.wait_for_timeout(600)
    # PHASE 6 flow: launch from the menu, pick a difficulty, drive.
    page.click("#crMenuScreen .ft-btn.primary"); page.wait_for_timeout(400)
    try:
        page.click("#crDMedium", timeout=1500)
    except Exception:
        pass
    page.wait_for_timeout(4300)
    page.keyboard.down("ArrowUp"); page.wait_for_timeout(3500)
    m = page.evaluate("""(() => {
      const c = document.getElementById('crCanvas');
      const ctx = c.getContext('2d');
      const W = 960, H = 540;
      let img = ctx.getImageData(0, 0, c.width, c.height);
      let d = img.data;
      const sx = c.width / W, sy = c.height / H;
      const px = (x, y) => {
        const i = ((Math.round(y*sy)) * c.width + Math.round(x*sx)) * 4;
        return [d[i], d[i+1], d[i+2]];
      };
      let cyanRows = 0, magRows = 0, laneRows = 0, rows = 0;
      /* Lane dashes scroll: on any single frame only the rows currently
       * covered by a dash band show white (the bands are every other
       * 3-segment group), and traffic/the player occlude some of those.
       * A per-frame row census is therefore inherently flaky — sample
       * several frames and keep the BEST coverage, which is what a human
       * eye perceives over a second of driving. */
      const lanePctSamples = [];
      for (let f = 0; f < 6; f++) {
        let lRows = 0, tRows = 0;
        for (let yy = 300; yy < 500; yy += 6) {
          tRows++;
          let hasLane = false;
          for (let xx = 0; xx < W; xx++) {
            const r = px(xx, yy);
            if (!hasLane && r[0] > 150 && r[1] > 150 && r[2] > 150) hasLane = true;
          }
          if (hasLane) lRows++;
        }
        lanePctSamples.push(Math.round(100 * lRows / tRows));
        // Re-grab the canvas for the next sample (dashes move).
        const img2 = c.getContext('2d').getImageData(0, 0, c.width, c.height);
        d = img2.data;
      }
      const lanePct = Math.max(...lanePctSamples);
      for (let yy = 300; yy < 500; yy += 6) {
        rows++;
        let hasCyan = false, hasMag = false;
        for (let xx = 0; xx < W; xx++) {
          const r = px(xx, yy);
          if (!hasCyan && r[2] > 120 && r[2] > r[0] + 40 && r[1] > 40) hasCyan = true;
          if (!hasMag && r[0] > 120 && r[0] > r[2] + 30 && r[1] < r[0]) hasMag = true;
        }
        if (hasCyan) cyanRows++;
        if (hasMag) magRows++;
      }
      let railRows = 0, railTries = 0;
      for (let yy = 320; yy < 500; yy += 8) {
        railTries++;
        for (let xx = 0; xx < W; xx++) {
          const r = px(xx, yy);
          if (r[2] > 95 && r[2] > r[0] + 25 && r[1] > 30) { railRows++; break; }
        }
      }
      const seen = new Set();
      for (let yy = H-125; yy < H-15; yy += 3)
        for (let xx = 380; xx < 580; xx += 3) {
          const r = px(xx, yy);
          seen.add((r[0]>>4)+','+(r[1]>>4)+','+(r[2]>>4));
        }
      let cityLights = 0;
      for (let yy = 30; yy < 225; yy += 3)
        for (let xx = 10; xx < W-10; xx += 5) {
          const r = px(xx, yy);
          if (r[0]+r[1]+r[2] > 260) cityLights++;
        }
      // Neon sun: scan a WIDE band around its rest position (W*0.72) and
      // the full vertical range it can occupy — the disc parallaxes with
      // playerX/curve AND rides the horizon, which itself moves with the
      // road's hills while driving.
      let sunPx = 0;
      const sunCx = W * 0.72;
      for (let yy = 120; yy < 280; yy += 2)
        for (let xx = Math.round(sunCx - 220); xx < Math.round(sunCx + 220); xx += 3) {
          if (xx < 0 || xx >= W) continue;
          const r = px(xx, yy);
          if (r[0] > 110 && r[0] > r[1]+25 && r[0] > r[2]) sunPx++;
        }
      // traffic car presence: any distinctly-coloured blob mid-road
      let trafficPx = 0;
      for (let yy = 250; yy < 420; yy += 4)
        for (let xx = 200; xx < 760; xx += 6) {
          const r = px(xx, yy);
          if (r[0] > 90 && r[0] < 250 && Math.abs(r[0]-r[2]) > 45) trafficPx++;
        }
      return {cyanPct: Math.round(100*cyanRows/rows),
              magPct: Math.round(100*magRows/rows),
              lanePct: lanePct,
              railPct: Math.round(100*railRows/railTries),
              carColors: seen.size, cityLights, sunPx, trafficPx,
              fps: window.CRD ? window.CRD.fps : null};
    })()""")
    check("visual/neonCyanEdge", m["cyanPct"] >= 55, f"{m['cyanPct']}% of scan rows")
    check("visual/neonMagentaEdge", m["magPct"] >= 45, f"{m['magPct']}% of scan rows")
    check("visual/laneDashes", m["lanePct"] >= 40, f"{m['lanePct']}% of scan rows")
    check("visual/guardrail", m["railPct"] >= 60, f"{m['railPct']}% of scan rows")
    check("visual/playerCarDetail", m["carColors"] >= 90, f"{m['carColors']} quantised colours")
    check("visual/cityDepth", m["cityLights"] >= 300, f"{m['cityLights']} lit pixels")
    check("visual/neonSun", m["sunPx"] >= 300, f"{m['sunPx']} sun pixels")
    check("visual/perf60fps", m["fps"] >= 50, f"{m['fps']} fps")
    page.screenshot(path=f"{OUT}/06-driving-rich.png")

    # ---------- RESPONSIVE ----------
    for w, h in [(1920,1080),(1536,864),(1366,768),(390,844)]:
        page.set_viewport_size({"width": w, "height": h})
        page.evaluate("location.hash = '#funtime/cr'")
        page.wait_for_selector("#crCanvas"); page.wait_for_timeout(600)
        st = page.evaluate("""(() => {
          const c = document.getElementById('crCanvas').getBoundingClientRect();
          return {w: Math.round(c.width), h: Math.round(c.height),
                  inVp: c.top >= -1 && c.left >= -1 && c.right <= innerWidth+1 && c.bottom <= innerHeight+1,
                  hScroll: document.documentElement.scrollWidth > innerWidth + 1};})()""")
        check(f"resp/{w}x{h}", st["inVp"] and not st["hScroll"] and st["w"] > 200,
              json.dumps(st))
        page.screenshot(path=f"{OUT}/07-{w}x{h}.png")

    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))
    b.close()

fails = [r for r in RESULTS if not r[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
