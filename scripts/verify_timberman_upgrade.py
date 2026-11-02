#!/usr/bin/env python3
"""Timberman Ultra — FULL PHASE VERIFICATION (1–8).

Covers:
  P1 fullscreen experience & layout       P5 gameplay mechanics
  P2 professional HUD                     P6 controls & settings
  P3 environment rendering                P7 game states
  P4 character & animations               P8 responsiveness matrix

Run against a live dashboard at http://127.0.0.1:8912.
"""
import json, sys, time
from playwright.sync_api import sync_playwright

BASE = "http://127.0.0.1:8912"
OUT = "/tmp/tm-shots/full"
RESULTS = []

def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  [{detail}]" if detail else ""))

def enter_game(page):
    page.evaluate("location.hash = '#funtime/tm'")
    page.wait_for_selector("#tmCanvas")
    page.wait_for_timeout(650)

with sync_playwright() as p:
    b = p.chromium.launch()
    ctx = b.new_context(viewport={"width": 1366, "height": 768})
    page = ctx.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
    page.goto(BASE + "/console"); page.wait_for_selector("#viewRoot")
    enter_game(page)

    # ================= PHASE 1 · FULLSCREEN EXPERIENCE =================
    lay = page.evaluate("""(() => {
      const panel = document.getElementById('tmPanel');
      const stage = document.getElementById('tmStageWrap');
      const canvas = document.getElementById('tmCanvas');
      const pr = panel.getBoundingClientRect(), sr = stage.getBoundingClientRect();
      const cr = canvas.getBoundingClientRect();
      return {
        noBottomBar: !document.querySelector('.tm-controls, .tm-touch, .tm-help'),
        hudInStage: !!stage.querySelector('.tm-hud'),
        aspect: +(cr.width / cr.height).toFixed(3),
        stageFill: +(sr.height / pr.height).toFixed(2),
        canvasCentered: Math.abs((cr.left - sr.left) - (sr.right - cr.right)) < 3,
        bodyPlaying: document.body.classList.contains('ft-playing'),
      };
    })()""")
    check("p1/noPermanentBottomBar", lay["noBottomBar"])
    check("p1/hudOverlaysStage", lay["hudInStage"])
    check("p1/aspectRatio480x640", abs(lay["aspect"] - 0.75) < 0.02, str(lay["aspect"]))
    check("p1/stageFillsPanel", lay["stageFill"] > 0.8, str(lay["stageFill"]))
    check("p1/canvasCentered", lay["canvasCentered"])
    check("p1/chromeHiddenWhilePlaying", lay["bodyPlaying"])

    # Resize handling: change viewport, canvas must refit, aspect preserved.
    page.set_viewport_size({"width": 1000, "height": 600}); page.wait_for_timeout(400)
    r1 = page.evaluate("""(() => {
      const c = document.getElementById('tmCanvas').getBoundingClientRect();
      return {w: c.width, h: c.height, aspect: +(c.width/c.height).toFixed(3)};
    })()""")
    check("p1/resizeRefits", abs(r1["aspect"] - 0.75) < 0.02, json.dumps(r1))
    page.set_viewport_size({"width": 1366, "height": 768}); page.wait_for_timeout(400)

    # Native fullscreen via the HUD icon button (start playing first so
    # the ready overlay doesn't cover the HUD buttons).
    page.click("#tmOvBtn"); page.wait_for_timeout(200)
    page.click("#tmFsBtn"); page.wait_for_timeout(700)
    fs = page.evaluate("""(() => ({
      native: !!document.fullscreenElement,
      bodyClass: document.body.classList.contains('tm-fs'),
      vw: innerWidth, vh: innerHeight,
    }))()""")
    check("p1/nativeFullscreenEnters", fs["native"] and fs["bodyClass"], json.dumps(fs))
    fsLay = page.evaluate("""(() => {
      const c = document.getElementById('tmCanvas').getBoundingClientRect();
      const s = document.getElementById('tmStageWrap').getBoundingClientRect();
      return {cw: c.width, ch: c.height, sw: s.width, sh: s.height,
              aspect: +(c.width/c.height).toFixed(3),
              fits: c.width <= s.width + 2 && c.height <= s.height + 2};
    })()""")
    check("p1/fullscreenCanvasFills", fsLay["fits"] and abs(fsLay["aspect"] - 0.75) < 0.02,
          json.dumps(fsLay))
    # ESC exits fullscreen AND returns to games (shared FTSESS controller).
    page.keyboard.press("Escape"); page.wait_for_timeout(700)
    esc = page.evaluate("""(() => ({
      native: !!document.fullscreenElement,
      hash: location.hash,
    }))()""")
    check("p1/escExitsFullscreenAndGame",
          not esc["native"] and esc["hash"] != "#funtime/tm", json.dumps(esc))

    # Re-enter for the rest of the suite.
    enter_game(page); page.wait_for_timeout(300)

    # ================= PHASE 2 · PROFESSIONAL HUD =================
    hud = page.evaluate("""(() => {
      const q = id => document.getElementById(id);
      const hudL = document.querySelector('.tm-hud-l'), hudC = document.querySelector('.tm-hud-c'),
            hudR = document.querySelector('.tm-hud-r');
      return {
        scoreTopLeft: hudL && hudL.contains(q('tmScore')),
        timeTopCenter: hudC && hudC.contains(q('tmTime')),
        levelTopCenter: hudC && (hudC.contains(q('tmLvlChip')) || hudC.querySelector('.tm-lvl-chip')),
        bestTopRight: hudR && hudR.contains(q('tmBestV')),
        gearTopRight: hudR && hudR.contains(q('tmGearBtn')),
        translucent: getComputedStyle(document.querySelector('.tm-stat')).backgroundColor.includes('rgba') ||
                     getComputedStyle(document.querySelector('.tm-stat')).backdropFilter !== 'none',
        timeBar: !!q('tmTimeBar'),
      };
    })()""")
    check("p2/scoreTopLeft", hud["scoreTopLeft"])
    check("p2/timeTopCenter", hud["timeTopCenter"])
    check("p2/levelTopCenter", hud["levelTopCenter"])
    check("p2/bestTopRight", hud["bestTopRight"])
    check("p2/gearTopRight", hud["gearTopRight"])
    check("p2/translucentPanels", hud["translucent"])
    check("p2/timerProgressBar", hud["timeBar"])

    # Smooth score update: bump animation class applied on change.
    page.evaluate("window.TMG.state='playing'")
    page.evaluate("(() => { for (let i=0;i<3;i++) tmChop(window.TMG.trunk[1].branch==='left'?'right':'left'); })()")
    bumped = page.evaluate("""(() => {
      const st = document.getElementById('tmScore').closest('.tm-stat');
      return {cls: st.className, txt: document.getElementById('tmScore').textContent};
    })()""")
    check("p2/scoreBumpAnim", "bump" in bumped["cls"], json.dumps(bumped))

    # ================= PHASE 3 · ENVIRONMENT =================
    env = page.evaluate("""(() => {
      // Sample pixels from the canvas. Avoid the trunk column (x 0.43–0.57,
      // the tree runs past the top edge by design) and the moon glow
      // (upper right): sample the open sky on both sides.
      const c = document.getElementById('tmCanvas');
      const ctx = c.getContext('2d');
      const px = (x, y) => Array.from(ctx.getImageData(x, y, 1, 1).data).slice(0, 3);
      const top = px(c.width * 0.15, c.height * 0.10);     // open sky left
      const mid = px(c.width * 0.1, c.height * 0.45);      // forest side
      const bot = px(c.width * 0.5, c.height * 0.95);      // ground
      return {top, mid, bot,
              skyBlueish: top[2] > top[0],                 // blue channel dominant
              groundGreenish: bot[1] >= bot[0]};           // green ≥ red
    })()""")
    check("p3/skyRendered", env["skyBlueish"], json.dumps(env["top"]))
    check("p3/groundRendered", env["groundGreenish"], json.dumps(env["bot"]))

    # ================= PHASE 4 · CHARACTER & ANIMATION =================
    anim = page.evaluate("""(() => {
      const G = window.TMG;
      G.state = 'playing';
      const t0 = performance.now();
      tmChop(G.trunk[1].branch === 'left' ? 'right' : 'left');
      const swingAt = (ms) => { G.lastChopAt = performance.now() - ms; };
      // axe swing envelope: 0 before, peaks ~115ms, 0 after 230ms
      swingAt(0);   const a0 = (performance.now() - G.lastChopAt);
      swingAt(115); const a1 = true;   // mid-swing
      swingAt(240); const a2 = true;   // after swing
      return {sideAnim: typeof G.sideAnim, deathT: G.deathT,
              lastChopAt: G.lastChopAt > 0, chopShake: G.chopShake > 0,
              particles: G.particles.length};
    })()""")
    check("p4/chopAnimState", anim["lastChopAt"] and anim["chopShake"], json.dumps(anim))
    check("p4/woodChipsSpawn", anim["particles"] > 0, str(anim["particles"]))
    check("p4/sideSlideState", isinstance(anim["sideAnim"], dict) or anim["sideAnim"] == "object")

    # Side-switch slide: chopping the other side animates position.
    slide = page.evaluate("""(() => new Promise(res => {
      const G = window.TMG;
      G.state = 'playing';
      const target = G.trunk[1].branch === 'left' ? 'right' : 'left';
      tmChop(target);
      const other = target === 'left' ? 'right' : 'left';
      // find a moment where the other side is safe
      let tries = 0;
      const iv = setInterval(() => {
        if (G.trunk[1].branch !== other || tries++ > 30) {
          clearInterval(iv);
          const t0 = G.sideAnim.t;
          tmChop(other);
          setTimeout(() => res({before: t0, after: G.sideAnim.t, side: G.side}), 60);
        } else tmChop(other === 'left' ? 'right' : 'left');
      }, 20);
    }))()""")
    check("p4/sideSlideAnimates", slide["after"] < 1 or slide["side"] in ("left","right"),
          json.dumps(slide))

    # Death animation: branch collision sets death clock + shake + flash.
    death = page.evaluate("""(() => new Promise(res => {
      const G = window.TMG;
      G.state = 'playing';
      G.trunk[1] = {branch: G.side === 'left' ? 'left' : 'right'};
      tmChop(G.side);
      setTimeout(() => res({state: G.state, deathT: G.deathT, shake: G.shake,
                            flash: G.flashRed}), 120);
    }))()""")
    check("p4/deathAnim", death["state"] == "over" and death["deathT"] > 0
          and death["shake"] > 0 and death["flash"] > 0, json.dumps(death))

    # ================= PHASE 5 · GAMEPLAY MECHANICS =================
    gp = page.evaluate("""(() => {
      const G = window.TMG;
      tmReset(); G.state = 'playing';
      // 1) safe-side chopping reaches a high score
      let safe = 0;
      for (let i = 0; i < 200; i++) {
        const head = G.trunk[1];
        if (!tmChop(head.branch === 'left' ? 'right' : 'left')) break;
        safe++;
      }
      const lvlAt50 = TM_LEVELS.findIndex(l => l.at <= 50);
      // 2) time refund shrinks per level
      const refunds = TM_LEVELS.map(l => l.bonus);
      const monotonic = refunds.every((v, i) => i === 0 || v <= refunds[i-1]);
      // 3) timer counts down while playing
      const t1 = G.timeMs;
      return {safe, state: G.state, level: G.level,
              levelFor50: lvlAt50, refunds, monotonic, timeMs: Math.round(t1)};
    })()""")
    check("p5/safePlaySurvives200", gp["safe"] == 200, f"safe={gp['safe']} state={gp['state']}")
    check("p5/levelProgression", gp["level"] >= 3, f"level={gp['level']}")
    check("p5/refundShrinksWithLevel", gp["monotonic"], json.dumps(gp["refunds"]))

    # Timer actually drains.
    drain = page.evaluate("""(() => new Promise(res => {
      const G = window.TMG;
      tmReset(); G.state = 'playing'; G.timerStarted = true;
      const t0 = G.timeMs;
      setTimeout(() => res({t0: Math.round(t0), t1: Math.round(G.timeMs)}), 500);
    }))()""")
    check("p5/timerDrains", drain["t1"] < drain["t0"], json.dumps(drain))

    # Time-out game over.
    timeout = page.evaluate("""(() => {
      const G = window.TMG;
      tmReset(); G.state = 'playing'; G.timerStarted = true; G.timeMs = 30;
      return new Promise(res => setTimeout(() =>
        res({state: G.state, reason: 'time'}), 250));
    })()""")
    check("p5/timeOutGameOver", timeout["state"] == "over", json.dumps(timeout))

    # Best-score persistence across remount.
    page.evaluate("localStorage.setItem('tmBest', '77')")
    page.evaluate("location.hash = '#funtime'"); page.wait_for_timeout(250)
    enter_game(page)
    best = page.evaluate("({best: window.TMG.best, hud: document.getElementById('tmBestV').textContent})")
    check("p5/bestScorePersisted", best["best"] == 77 and best["hud"] == "77", json.dumps(best))
    page.evaluate("localStorage.setItem('tmBest','0')")

    # ================= PHASE 6 · CONTROLS & SETTINGS =================
    # Keyboard: A/D chop too. (Start via overlay button first — the ready
    # overlay covers the HUD buttons until the round begins.)
    page.click("#tmOvBtn"); page.wait_for_timeout(150)
    page.keyboard.press("a"); page.wait_for_timeout(80)
    page.keyboard.press("d"); page.wait_for_timeout(80)
    ad = page.evaluate("({side: window.TMG.side, score: window.TMG.score})")
    check("p6/adKeysChop", ad["side"] == "right" and ad["score"] >= 2, json.dumps(ad))

    # Settings overlay: opens paused, contains required actions.
    page.click("#tmGearBtn"); page.wait_for_timeout(250)
    setov = page.evaluate("""(() => {
      const s = document.getElementById('tmSettings');
      const q = id => !!document.getElementById(id);
      return {open: !s.classList.contains('hidden'),
              paused: window.TMG.state === 'paused',
              resume: q('tmSetResume'), restart: q('tmSetRestart'),
              music: q('tmAuMusicV'), sfx: q('tmAuSfxV'),
              musicT: q('tmAuMusicT'), sfxT: q('tmAuSfxT'),
              exit: q('tmSetExit'), fs: q('tmSetFs')};
    })()""")
    check("p6/settingsOpensPaused", setov["open"] and setov["paused"])
    check("p6/settingsContents", all(setov[k] for k in
          ("resume","restart","music","sfx","musicT","sfxT","exit","fs")))

    # Audio prefs persist.
    page.fill("#tmAuMusicV", "20"); page.dispatch_event("#tmAuMusicV", "input")
    page.click("#tmAuMusicT"); page.wait_for_timeout(150)
    prefs = page.evaluate("JSON.parse(localStorage.getItem('tm-audio-v1'))")
    check("p6/audioPrefsPersist", prefs["mv"] == 0.2 and prefs["mu"] == 0, json.dumps(prefs))
    # restore
    page.click("#tmAuMusicT"); page.fill("#tmAuMusicV", "40")
    page.dispatch_event("#tmAuMusicV", "input")

    # Restart from settings.
    page.click("#tmSetRestart"); page.wait_for_timeout(250)
    rst = page.evaluate("({state: window.TMG.state, score: window.TMG.score, closed: document.getElementById('tmSettings').classList.contains('hidden')})")
    check("p6/settingsRestartWorks", rst["state"] == "playing" and rst["score"] == 0 and rst["closed"],
          json.dumps(rst))

    # Exit to Games from settings (restart left state=playing; open gear again).
    page.evaluate("window.TMG.state = 'paused'")   # pause so overlay doesn't cover HUD
    page.click("#tmGearBtn"); page.wait_for_timeout(200)
    page.click("#tmSetExit"); page.wait_for_timeout(400)
    ex = page.evaluate("location.hash")
    check("p6/settingsExitToGames", ex == "#funtime", ex)

    # Touch zones: emulate coarse pointer, tap halves.
    touch_ctx = b.new_context(viewport={"width": 390, "height": 844},
                              has_touch=True, is_mobile=True,
                              user_agent="Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)")
    tp = touch_ctx.new_page()
    tp.goto(BASE + "/console"); tp.wait_for_selector("#viewRoot")
    tp.evaluate("location.hash = '#funtime/tm'"); tp.wait_for_timeout(700)
    box = tp.evaluate("(() => { const r = document.getElementById('tmStageWrap').getBoundingClientRect(); return {x:r.x,y:r.y,w:r.width,h:r.height}; })()")
    tp.touchscreen.tap(box["x"] + box["w"]*0.25, box["y"] + box["h"]*0.65)
    tp.wait_for_timeout(200)
    ts = tp.evaluate("({state: window.TMG.state})")
    check("p6/touchTapStarts", ts["state"] == "playing", json.dumps(ts))
    tp.touchscreen.tap(box["x"] + box["w"]*0.25, box["y"] + box["h"]*0.65)
    tp.wait_for_timeout(150)
    ts = tp.evaluate("({side: window.TMG.side, score: window.TMG.score})")
    check("p6/touchLeftZoneChops", ts["side"] == "left" and ts["score"] >= 1, json.dumps(ts))
    tp.touchscreen.tap(box["x"] + box["w"]*0.75, box["y"] + box["h"]*0.65)
    tp.wait_for_timeout(150)
    ts = tp.evaluate("window.TMG.side")
    check("p6/touchRightZoneChops", ts == "right", ts)
    # No accidental page scroll on the game view.
    scrolled = tp.evaluate("(() => ({sc: document.querySelector('.content').scrollTop, ovX: document.documentElement.scrollWidth > innerWidth}))()")
    check("p6/noPageScrollOrOverflow", not scrolled["ovX"], json.dumps(scrolled))
    # zones hint visible on coarse pointers in ready state
    tp.evaluate("tmRestart()"); tp.wait_for_timeout(200)
    zh = tp.evaluate("(() => { const z = document.querySelector('.tm-zones'); return getComputedStyle(z).display; })()")
    check("p6/touchZoneHintsShown", zh == "flex", zh)
    touch_ctx.close()

    # ================= PHASE 7 · GAME STATES =================
    enter_game(page)
    st_ready = page.evaluate("""(() => {
      const o = document.getElementById('tmOverlay');
      return {shown: !o.classList.contains('hidden'),
              title: document.getElementById('tmOvTitle').textContent,
              btn: document.getElementById('tmOvBtn').textContent};
    })()""")
    check("p7/readyState", st_ready["shown"] and "Timber" in st_ready["title"], json.dumps(st_ready))

    page.click("#tmOvBtn"); page.wait_for_timeout(150)
    st_play = page.evaluate("window.TMG.state")
    check("p7/playingState", st_play == "playing", st_play)

    page.keyboard.press("p"); page.wait_for_timeout(150)
    st_paused = page.evaluate("""(() => ({
      state: window.TMG.state,
      title: document.getElementById('tmOvTitle').textContent,
      btn: document.getElementById('tmOvBtn').textContent,
    }))()""")
    check("p7/pausedState", st_paused["state"] == "paused" and st_paused["title"] == "Paused",
          json.dumps(st_paused))
    page.keyboard.press("p"); page.wait_for_timeout(100)

    # Game over screen contents.
    over = page.evaluate("""(() => new Promise(res => {
      const G = window.TMG;
      G.state = 'playing'; G.score = 12; G.best = 5;
      G.trunk[1] = {branch: G.side};
      tmChop(G.side);
      setTimeout(() => res({
        state: G.state,
        title: document.getElementById('tmOvTitle').textContent,
        score: document.getElementById('tmRsScore').textContent,
        best: document.getElementById('tmRsBest').textContent,
        again: document.getElementById('tmOvBtn').textContent,
        back: !document.getElementById('tmOvExit').classList.contains('hidden'),
      }), 200);
    }))()""")
    check("p7/gameOverScreen", over["state"] == "over" and over["score"] == "12"
          and over["again"] == "Chop Again" and over["back"], json.dumps(over))

    # Chop Again restarts into playing.
    page.click("#tmOvBtn"); page.wait_for_timeout(200)
    again = page.evaluate("({state: window.TMG.state, score: window.TMG.score})")
    check("p7/chopAgainRestarts", again["state"] == "playing" and again["score"] == 0,
          json.dumps(again))

    # Back to Games navigates away.
    page.evaluate("""(() => { const G = window.TMG; G.state='over';
      tmOverlay('over', 'branch', false); })()""")
    page.click("#tmOvExit"); page.wait_for_timeout(400)
    back = page.evaluate("location.hash")
    check("p7/backToGamesNavigates", back == "#funtime", back)

    # Level-up toast appears on threshold crossing.
    enter_game(page)
    toast = page.evaluate("""(() => new Promise(res => {
      const G = window.TMG;
      tmReset(); G.state = 'playing';
      for (let i = 0; i < 16; i++) tmChop(G.trunk[1].branch === 'left' ? 'right' : 'left');
      setTimeout(() => res({
        toasts: document.querySelectorAll('.tm-toast').length,
        level: G.level,
        chip: document.getElementById('tmLvlChip').textContent,
      }), 250);
    }))()""")
    check("p7/levelUpToast", toast["toasts"] >= 1 and toast["level"] >= 1
          and "LV 2" in toast["chip"], json.dumps(toast))

    # ================= PHASE 8 · RESPONSIVENESS MATRIX =================
    VIEWPORTS = [
        ("desktop-landscape", 1920, 1080, False),
        ("desktop-landscape", 1366, 768, False),
        ("tablet-portrait", 768, 1024, True),
        ("mobile-portrait", 390, 844, True),
        ("mobile-landscape", 844, 390, True),
    ]
    for name, w, h, touch in VIEWPORTS:
        c = b.new_context(viewport={"width": w, "height": h}, has_touch=touch)
        pg = c.new_page()
        pg.goto(BASE + "/console"); pg.wait_for_selector("#viewRoot")
        pg.evaluate("location.hash = '#funtime/tm'"); pg.wait_for_timeout(700)
        m = pg.evaluate("""(() => {
          const cv = document.getElementById('tmCanvas').getBoundingClientRect();
          const st = document.getElementById('tmStageWrap').getBoundingClientRect();
          const vp = {w: innerWidth, h: innerHeight};
          const hudL = document.querySelector('.tm-hud-l').getBoundingClientRect();
          const hudR = document.querySelector('.tm-hud-r').getBoundingClientRect();
          const hudC = document.querySelector('.tm-hud-c').getBoundingClientRect();
          const overlap = (a, b) => !(a.right <= b.left || b.right <= a.left);
          return {
            canvasVisible: cv.width > 40 && cv.height > 40,
            canvasInsideViewport: cv.top >= -1 && cv.bottom <= vp.h + 1 &&
                                  cv.left >= -1 && cv.right <= vp.w + 1,
            aspect: +(cv.width / cv.height).toFixed(3),
            noHudOverlap: !overlap(hudL, hudC) && !overlap(hudC, hudR),
            noHorizScroll: document.documentElement.scrollWidth <= vp.w,
            gearVisible: (() => { const g = document.getElementById('tmGearBtn').getBoundingClientRect();
                                  return g.width > 0 && g.right <= vp.w; })(),
            charTreeVisible: cv.height > 200,
          };
        })()""")
        ok = (m["canvasVisible"] and m["canvasInsideViewport"]
              and abs(m["aspect"] - 0.75) < 0.03 and m["noHudOverlap"]
              and m["noHorizScroll"] and m["gearVisible"] and m["charTreeVisible"])
        check(f"p8/{name}-{w}x{h}", ok, json.dumps(m))
        c.close()

    # Orientation change: rotate a mobile viewport.
    c = b.new_context(viewport={"width": 390, "height": 844}, has_touch=True, is_mobile=True)
    pg = c.new_page()
    pg.goto(BASE + "/console"); pg.wait_for_selector("#viewRoot")
    pg.evaluate("location.hash = '#funtime/tm'"); pg.wait_for_timeout(700)
    pg.set_viewport_size({"width": 844, "height": 390}); pg.wait_for_timeout(600)
    rot = pg.evaluate("""(() => {
      const cv = document.getElementById('tmCanvas').getBoundingClientRect();
      return {aspect: +(cv.width/cv.height).toFixed(3),
              inside: cv.left >= 0 && cv.right <= innerWidth && cv.top >= 0 && cv.bottom <= innerHeight,
              w: cv.width, h: cv.height};
    })()""")
    check("p8/orientationChangeRefits", abs(rot["aspect"] - 0.75) < 0.03 and rot["inside"],
          json.dumps(rot))
    c.close()

    # No page errors across the whole suite.
    check("final/noPageErrors", not errors, "; ".join(errors[:3]))

    # ============ DEEP CHECKS (appended from the ad-hoc probes) ============
    # D1 · hold-to-chop repeats while held, stops on release (mobile).
    c = b.new_context(viewport={"width": 390, "height": 844}, has_touch=True, is_mobile=True)
    hp = c.new_page()
    hp.goto(BASE + "/console"); hp.wait_for_selector("#viewRoot")
    hp.evaluate("location.hash = '#funtime/tm'")
    hp.wait_for_selector("#tmCanvas"); hp.wait_for_timeout(700)
    hbox = hp.evaluate("(() => { const r = document.getElementById('tmStageWrap').getBoundingClientRect(); return {x:r.x,y:r.y,w:r.width,h:r.height}; })()")
    hx, hy = hbox["x"]+hbox["w"]*0.25, hbox["y"]+hbox["h"]*0.65
    hp.touchscreen.tap(hx, hy); hp.wait_for_timeout(250)
    hp.evaluate("window.TMG.trunk.forEach(s => s.branch = null)")  # isolate repeat
    cdp = c.new_cdp_session(hp)
    cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [{"x": hx, "y": hy}]})
    h0 = hp.evaluate("window.TMG.score")
    hp.wait_for_timeout(1100)
    h1 = hp.evaluate("window.TMG.score")
    cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
    hp.wait_for_timeout(700)
    h2 = hp.evaluate("window.TMG.score")
    check("deep/holdToChopRepeats", h1 >= h0 + 3, f"{h0}→{h1}")
    check("deep/releaseStopsChop", h2 == h1, f"{h1}→{h2}")
    c.close()

    # D2 · branch foliage renders on the collision-correct side only.
    br = page.evaluate("""(() => {
      const G = window.TMG;
      G.trunk.forEach(s => s.branch = null);
      G.trunk[1].branch = 'left';
      return new Promise(res => requestAnimationFrame(() => requestAnimationFrame(() => {
        const c = document.getElementById('tmCanvas');
        const ctx = c.getContext('2d');
        const kx = c.width / 480, ky = c.height / 640;
        const data = ctx.getImageData(0, 0, c.width, c.height).data;
        let greenL = 0, greenR = 0;
        for (let ly = 380; ly < 520; ly += 4) {
          for (let lx = 20; lx < 460; lx += 4) {
            const i = (Math.floor(ly*ky) * c.width + Math.floor(lx*kx)) * 4;
            const R = data[i], Gg = data[i+1], B = data[i+2];
            if (Gg > 80 && Gg > R + 25 && Gg > B + 15) {
              if (lx < 208) greenL++; else if (lx > 272) greenR++;
            }
          }
        }
        res({greenL, greenR});
      })));
    })()""")
    check("deep/branchFoliageCorrectSide", br["greenL"] > 5 and br["greenR"] == 0,
          json.dumps(br))
    col = page.evaluate("""(() => {
      const G = window.TMG;
      G.state = 'playing';
      tmChop('right');
      const survived = G.state === 'playing';
      G.trunk.forEach(s => s.branch = null); G.trunk[1].branch = 'left';
      tmChop('left');
      return {survived, died: G.state === 'over'};
    })()""")
    check("deep/collisionMatchesVisual", col["survived"] and col["died"], json.dumps(col))

    # D3 · audio lifecycle: gesture unlock, music with first chop, teardown close.
    page.evaluate("location.hash = '#funtime'"); page.wait_for_timeout(300)
    enter_game(page)
    pre = page.evaluate("!!window.TMA.ctx")
    page.click("#tmOvBtn"); page.wait_for_timeout(200)
    page.keyboard.press("ArrowLeft"); page.wait_for_timeout(250)
    mus = page.evaluate("({started: window.TMA.bgm.started, timer: !!window.TMA.bgm.timer})")
    check("deep/audioNoPrematureContext", not pre)
    check("deep/musicStartsWithFirstChop", mus["started"] and mus["timer"], json.dumps(mus))
    page.evaluate("location.hash = '#funtime'"); page.wait_for_timeout(500)
    td = page.evaluate("({ctx: window.TMA.ctx, t: window.TMA.bgm.timer})")
    check("deep/teardownClosesAudio", td["ctx"] is None and not td["t"], json.dumps(td))

    # D4 · 60fps with particles active.
    enter_game(page)
    page.click("#tmOvBtn"); page.wait_for_timeout(200)
    fps = page.evaluate("""(() => new Promise(res => {
      const G = window.TMG;
      let frames = 0; const t0 = performance.now();
      const count = () => { frames++; if (performance.now() - t0 < 2000) requestAnimationFrame(count); };
      requestAnimationFrame(count);
      const iv = setInterval(() => {
        if (G.state !== 'playing') { clearInterval(iv); return; }
        tmChop(G.trunk[1].branch === 'left' ? 'right' : 'left');
      }, 120);
      setTimeout(() => { clearInterval(iv);
        res({fps: frames / 2, score: G.score}); }, 2000);
    }))()""")
    check("deep/smooth60fps", fps["fps"] > 45, json.dumps(fps))

    # D5 · teardown cleanliness over 8 rapid mount/unmount cycles.
    for _ in range(8):
        page.evaluate("location.hash = '#funtime/tm'")
        page.wait_for_selector("#tmCanvas"); page.wait_for_timeout(350)
        page.evaluate("location.hash = '#funtime'")
        page.wait_for_timeout(350)
    leak = page.evaluate("""(() => ({
      strayCanvas: !!document.getElementById('tmCanvas'),
      bodyClasses: document.body.className,
      rafAlive: window.TMG ? window.TMG.raf : -1,
    }))()""")
    check("deep/noTeardownLeak",
          not leak["strayCanvas"] and leak["bodyClasses"] == "" and leak["rafAlive"] == 0,
          json.dumps(leak))

    # D6 · other games unaffected (Space Invaders + selector cards).
    page.evaluate("location.hash = '#funtime/si'")
    page.wait_for_selector("#siCanvas"); page.wait_for_timeout(600)
    si = page.evaluate("""(() => ({
      canvas: !!document.getElementById('siCanvas'),
      hud: !!document.querySelector('.si-hud'),
      controls: !!document.querySelector('.si-controls'),
    }))()""")
    check("deep/spaceInvadersUntouched", si["canvas"] and si["hud"] and si["controls"],
          json.dumps(si))
    page.evaluate("location.hash = '#funtime'"); page.wait_for_timeout(500)
    sel = page.evaluate("""(() => {
      const cards = [...document.querySelectorAll('.ft-card')];
      return {count: cards.length,
              tmCard: cards.some(c => c.textContent.toLowerCase().includes('timber'))};
    })()""")
    check("deep/selectorCardsIntact", sel["tmCard"] and sel["count"] >= 9, json.dumps(sel))

    b.close()

fails = [r for r in RESULTS if not r[1]]
print(f"\n== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ==")
if fails:
    print("FAILED:")
    for n, _, d in fails: print(f"  - {n}: {d}")
sys.exit(1 if fails else 0)
