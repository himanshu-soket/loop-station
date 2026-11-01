#!/usr/bin/env python3
"""PHASE 9 QA: responsive + fullscreen matrix for Snakes & Ladders.
Checks at every viewport (windowed AND fullscreen):
  - board is square, reasonably large, fully inside the viewport
  - no horizontal page scrollbar
  - all controls visible & clickable (nothing clipped)
  - sidebar reachable
"""
import json, sys, time
from playwright.sync_api import sync_playwright

BASE = "http://127.0.0.1:8912"
OUT = "/tmp/sn-shots/after"
RESULTS = []

def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  [{detail}]" if detail else ""))

VIEWPORTS = [(1920,1080),(1536,864),(1366,768),(1280,720),(1024,768),(768,1024),(390,844)]

with sync_playwright() as p:
    b = p.chromium.launch()
    page = b.new_page(viewport={"width":1366,"height":768})
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(BASE + "/console"); page.wait_for_selector("#viewRoot")

    def enter_game():
        page.evaluate("location.hash = '#funtime/sn'")
        page.wait_for_selector("#snBoard")
        page.wait_for_timeout(500)

    enter_game()
    # set up a 4P named game once (persists via SNG across re-entries)
    page.click("#snModeSeg button[data-v='4p']"); page.wait_for_timeout(250)
    for i, nm in enumerate(["Himanshu","Rahul","Aman","Priya"]):
        page.fill(f"#snName{i}", nm)
    page.click("#snStart"); page.wait_for_timeout(300)
    page.evaluate("window.SND.fast()")

    def audit(tag, fs=False):
        st = page.evaluate("""(() => {
          const vp = {w: innerWidth, h: innerHeight};
          const c = document.getElementById('snBoard').getBoundingClientRect();
          const side = document.querySelector('.sn-side').getBoundingClientRect();
          const ids = ['snStart','snRestartBtn','snRollBtn','snFsBtn','snExit','snSoundBtn',
                       'snModeSeg','snNames','snPlayers','snLog','snMsg','snTurnChip'];
          const vis = {};
          for (const id of ids) {
            const e = document.getElementById(id);
            if (!e) { vis[id] = 'missing'; continue; }
            const r = e.getBoundingClientRect();
            /* an EMPTY container (log before any move) legitimately has no
               box; only flag elements that are CLIPPED by the viewport. */
            const emptyOk = (id === 'snLog') && r.height === 0;
            vis[id] = (emptyOk ||
                       (r.width > 0 && r.height > 0 &&
                        r.right > 0 && r.bottom > 0 &&
                        r.left < vp.w && r.top < vp.h)) ? 'ok'
                      : (r.height === 0 ? 'empty' :
                         [Math.round(r.left),Math.round(r.top),Math.round(r.right),Math.round(r.bottom)].join(','));
          }
          return {
            vp,
            board: {w: Math.round(c.width), h: Math.round(c.height),
                    ratio: +(c.width / c.height).toFixed(3),
                    x: Math.round(c.x), y: Math.round(c.y),
                    right: Math.round(c.right), bottom: Math.round(c.bottom)},
            side: {w: Math.round(side.width), visible: side.right > 0 && side.left < vp.w},
            hScroll: document.documentElement.scrollWidth > vp.w + 1,
            vis};})()""")
        ok_board = (abs(st["board"]["ratio"] - 1) < 0.02
                    and st["board"]["x"] >= -1 and st["board"]["y"] >= -1
                    and st["board"]["right"] <= st["vp"]["w"] + 1
                    and st["board"]["bottom"] <= st["vp"]["h"] + 1)
        check(f"{tag}/boardSquareAndInsideVp", ok_board,
              f"{st['board']['w']}x{st['board']['h']} @({st['board']['x']},{st['board']['y']}) vp{st['vp']['w']}x{st['vp']['h']}")
        check(f"{tag}/noHorizontalScrollbar", not st["hScroll"])
        bad = {k: v for k, v in st["vis"].items() if v != "ok"}
        check(f"{tag}/allControlsVisible", not bad, json.dumps(bad))
        check(f"{tag}/sidebarReachable", st["side"]["visible"], f"w={st['side']['w']}")
        return st

    # ---- windowed matrix ----
    for w, h in VIEWPORTS:
        page.set_viewport_size({"width": w, "height": h})
        enter_game()
        if w == 390:
            dbg = page.evaluate("""(() => {
              const r = (sel) => { const e=document.querySelector(sel); if(!e) return null;
                const b=e.getBoundingClientRect(); return [Math.round(b.y),Math.round(b.bottom)]; };
              return {mode: window.SNG.mode, n: window.SNG.game.players.length,
                      hud: r('.sn-hud'), setup: r('.sn-setup'), ctlRow: r('.sn-ctl-row'),
                      stage: r('.sn-stage'), board: r('#snBoard'), side: r('.sn-side')};})()""")
            print("DBG-390:", json.dumps(dbg))
        st = audit(f"win-{w}x{h}")
        page.screenshot(path=f"{OUT}/matrix-win-{w}x{h}.png")

    # ---- fullscreen matrix ----
    for w, h in [(1920,1080),(1536,864),(1366,768),(390,844)]:
        page.set_viewport_size({"width": w, "height": h})
        enter_game()
        page.click("#snFsBtn"); page.wait_for_timeout(900)
        native = page.evaluate("!!document.fullscreenElement")
        audit(f"fs-{w}x{h}", fs=True)
        page.screenshot(path=f"{OUT}/matrix-fs-{w}x{h}.png")
        page.keyboard.press("Escape"); page.wait_for_timeout(700)
        # Esc must exit BOTH fullscreen and the game (session behaviour)
        st = page.evaluate("""(() => ({native: !!document.fullscreenElement,
          backAtGames: !!document.getElementById('ftCardSn'),
          bodyClean: document.body.className === ''}))()""")
        check(f"esc-{w}x{h}/exitsCleanly", (not st["native"]) and st["backAtGames"] and st["bodyClean"],
              json.dumps(st))

    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))
    b.close()

fails = [r for r in RESULTS if not r[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
