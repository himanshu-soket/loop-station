#!/usr/bin/env python3
"""Hack the Terminal — visual + functional verification.

Covers: responsive layout matrix (windowed, immersive fallback and native
fullscreen), terminal fill ratio, no horizontal overflow, no clipped
controls, internal log scrolling, all four missions end-to-end through the
real command engine, scoring/timer/hints/pause/restart/sound/fullscreen,
and the presentation contract (mission header band, no ASCII separators,
clue styling, command prefixes).

Run with the dashboard serving on BASE (default 127.0.0.1:39487):
    ./target/debug/loop-dashboard --no-auth --port 39487 --sys-demo &
    python3 scripts/verify_hack_terminal.py
"""
import json
import os
import sys

from playwright.sync_api import sync_playwright

BASE = os.environ.get("HT_BASE", "http://127.0.0.1:39487")
OUT = "/tmp/ht-shots"
RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  [{detail}]" if detail else ""))


VIEWPORTS = [(1920, 1080), (1680, 1050), (1440, 900), (1280, 720),
             (1024, 768), (768, 1024), (600, 900), (390, 844), (320, 700)]

CONTROL_IDS = ["htStartBtn", "htPauseBtn", "htRestartBtn", "htSoundBtn",
               "htFsBtn", "htExit", "htInput"]

LAYOUT_PROBE = """(() => {
  const vp = {w: innerWidth, h: innerHeight};
  const doc = document.documentElement;
  const panel = document.getElementById('htPanel');
  const term = document.querySelector('.ht-term');
  const out = document.getElementById('htOut');
  const pb = panel.getBoundingClientRect();
  const ob = out.getBoundingClientRect();
  const vis = {};
  for (const id of %s) {
    const e = document.getElementById(id);
    if (!e) { vis[id] = 'missing'; continue; }
    const r = e.getBoundingClientRect();
    vis[id] = (r.width > 0 && r.height > 0 &&
               r.right <= vp.w + 1 && r.bottom <= vp.h + 1 &&
               r.left >= -1 && r.top >= -1) ? 'ok' : 'clipped';
  }
  return {
    vp,
    hScroll: doc.scrollWidth > vp.w + 1,
    panel: {top: Math.round(pb.top), bottom: Math.round(pb.bottom),
            w: Math.round(pb.width), h: Math.round(pb.height)},
    out: {h: Math.round(ob.height),
          pctOfVp: +(ob.height / vp.h * 100).toFixed(1),
          scrollable: out.scrollHeight > out.clientHeight + 2,
          atBottom: out.scrollHeight - out.scrollTop - out.clientHeight < 6},
    vis,
  };
})()""" % json.dumps(CONTROL_IDS)


def enter_game(page):
    page.goto(BASE + "/console", wait_until="networkidle")
    page.evaluate("location.hash = '#funtime/ht'")
    page.wait_for_selector("#htPanel")
    page.wait_for_timeout(500)


def cmd(page, text):
    page.click("#htInput")
    page.keyboard.type(text)
    page.keyboard.press("Enter")
    page.wait_for_timeout(160)


def solve_all(page):
    """Drive the real engine through all four missions; returns final state."""
    verbs = ["unlock", "submit", "decode", "bypass"]
    for i in range(4):
        cmd(page, "start" if i == 0 else "next")
        cmd(page, "begin")
        # mission 4's solve() renders arrows; the checker wants the comma form
        ans = page.evaluate(
            "i => (i === 3 ? HT.puzzle.answer : HT_MISSIONS[HT.mission].solve(HT.puzzle))", i)
        cmd(page, f"{verbs[i]} {ans}")
    return page.evaluate("({screen: HT.screen, score: HT.score})")


def main():
    os.makedirs(OUT, exist_ok=True)
    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page(viewport={"width": 1440, "height": 900})
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))

        # ---------- LAYOUT MATRIX (windowed) ----------
        for w, h in VIEWPORTS:
            page.set_viewport_size({"width": w, "height": h})
            enter_game(page)
            st = page.evaluate(LAYOUT_PROBE)
            tag = f"{w}x{h}"
            check(f"layout/{tag}/noHorizontalOverflow", not st["hScroll"])
            check(f"layout/{tag}/panelFillsViewportHeight",
                  st["panel"]["bottom"] >= st["vp"]["h"] - 2 and st["panel"]["top"] <= 1,
                  f"panel {st['panel']['top']}..{st['panel']['bottom']} of {st['vp']['h']}")
            check(f"layout/{tag}/terminalUsesViewport",
                  st["out"]["pctOfVp"] >= 25,
                  f"output is {st['out']['pctOfVp']}% of viewport")
            bad = {k: v for k, v in st["vis"].items() if v != "ok"}
            check(f"layout/{tag}/allControlsVisible", not bad, json.dumps(bad))

        # ---------- LONG LOG: internal scroll, prompt stays visible ----------
        page.set_viewport_size({"width": 1440, "height": 900})
        enter_game(page)
        cmd(page, "start"); cmd(page, "begin")
        for _ in range(40):
            cmd(page, "status")
        st = page.evaluate(LAYOUT_PROBE)
        check("scroll/logScrollsInternally", st["out"]["scrollable"],
              f"scrollH vs clientH at {st['out']['pctOfVp']}% fill")
        check("scroll/autoScrollSticksToBottom", st["out"]["atBottom"])
        check("scroll/promptRowStillVisible", st["vis"]["htInput"] == "ok")

        # ---------- FULLSCREEN (native) ----------
        # Re-enter first: exiting native fullscreen triggers FTSESS's
        # "leave the game" semantics on some paths, so start from a fresh view.
        page.evaluate("location.hash = '#funtime'"); page.wait_for_timeout(300)
        enter_game(page)
        for _ in range(40):
            cmd(page, "status")
        page.evaluate("document.querySelector('#htPanel').requestFullscreen()")
        page.wait_for_timeout(800)
        st = page.evaluate(LAYOUT_PROBE)
        check("fullscreen/panelIsTheScreen",
              st["panel"]["h"] >= st["vp"]["h"] - 2, f"{st['panel']['h']}px of {st['vp']['h']}")
        check("fullscreen/terminalFillsScreen", st["out"]["pctOfVp"] >= 55,
              f"output is {st['out']['pctOfVp']}% of viewport")
        check("fullscreen/longLogStillScrollsInside", st["out"]["scrollable"])
        check("fullscreen/promptVisibleWithLongLog", st["vis"]["htInput"] == "ok")
        bad = {k: v for k, v in st["vis"].items() if v != "ok"}
        check("fullscreen/allControlsVisible", not bad, json.dumps(bad))
        page.screenshot(path=f"{OUT}/fullscreen-longlog.png")
        # Leave fullscreen via the API quietly (not Esc, which exits the game)
        page.evaluate("window.__htFsGuard = true")   # keep FTSESS from navigating
        page.evaluate("""(() => {
          // Suppress the session controller's leave-on-fullscreen-exit for
          // this one transition so the test can continue in windowed mode.
          const orig = window.ftSessionLeave;
          window.ftSessionLeave = () => {};
          document.exitFullscreen().finally(() => {
            setTimeout(() => { window.ftSessionLeave = orig; }, 500);
          });
        })()""")
        page.wait_for_timeout(900)

        # ---------- IMMERSIVE FALLBACK (CSS) ----------
        page.evaluate("document.body.classList.add('ft-immersive')")
        page.wait_for_timeout(300)
        st = page.evaluate(LAYOUT_PROBE)
        check("immersive/panelFitsViewport",
              st["panel"]["bottom"] <= st["vp"]["h"] + 1)
        check("immersive/terminalFills", st["out"]["pctOfVp"] >= 55,
              f"{st['out']['pctOfVp']}%")
        page.evaluate("document.body.classList.remove('ft-immersive')")

        # ---------- PRESENTATION CONTRACT ----------
        enter_game(page)
        head = page.eval_on_selector("#htMhead", "e => e.textContent")
        check("present/missionHeaderBandExists",
              head and "STANDBY" in head, repr(head))
        cmd(page, "start")
        head = page.eval_on_selector("#htMhead", "e => e.textContent")
        check("present/headerShowsMissionNumber", "LOCK 1/4" in head, repr(head))
        seps = page.evaluate(
            "[...document.querySelectorAll('#htOut .l-hd')]"
            ".filter(c => /[═─]{6,}/.test(c.textContent)).length")
        check("present/noAsciiSeparators", seps == 0, f"{seps} separator lines")
        cmd(page, "begin")
        clue_style = page.eval_on_selector(
            "#htOut .ht-clue", "e => getComputedStyle(e).paddingLeft")
        check("present/cluesIndentedGroup", clue_style not in ("0px", ""), clue_style)
        marker = page.eval_on_selector(
            "#htOut .ht-clue", "e => getComputedStyle(e, '::before').content")
        check("present/clueMarkerPainted", marker and marker not in ("none", '""'), str(marker))
        prefix = page.eval_on_selector(
            "#htOut .l-cmd", "e => getComputedStyle(e, '::before').content")
        check("present/commandPrefix", "❯" in str(prefix), str(prefix))

        # ---------- GAMEPLAY: all four missions ----------
        page.evaluate("location.hash = '#funtime'"); page.wait_for_timeout(300)
        enter_game(page)
        final = solve_all(page)
        check("gameplay/allFourMissionsComplete", final["screen"] == "victory",
              json.dumps(final))
        check("gameplay/perfectScoreIs2520", final["score"] == 2520,
              f"score {final['score']}")
        summary = page.evaluate(
            "[...document.querySelectorAll('#htOut .l-ok')]"
            ".map(e => e.textContent).find(t => t.includes('Final score'))")
        check("gameplay/victoryMaxLabelledCorrectly",
              summary and "2520" in summary, repr(summary))

        # ---------- CONTROLS ----------
        enter_game(page)
        page.click("#htStartBtn"); page.wait_for_timeout(200)
        page.click("#htStartBtn")   # brief screen -> begin
        page.wait_for_timeout(200)
        check("controls/startButtonDrivesEngine",
              page.evaluate("HT.screen") == "active")
        page.click("#htPauseBtn"); page.wait_for_timeout(150)
        check("controls/pauseFreezesTimer",
              page.evaluate("HT.paused") is True and
              page.eval_on_selector("#htPauseBtn", "e => e.textContent") == "Resume")
        page.click("#htPauseBtn"); page.wait_for_timeout(150)
        check("controls/resumeRestores",
              page.evaluate("HT.paused") is False and
              page.eval_on_selector("#htPauseBtn", "e => e.textContent") == "Pause")
        page.click("#htSoundBtn"); page.wait_for_timeout(150)
        check("controls/soundToggleSyncs",
              page.eval_on_selector("#htSoundBtn", "e => e.getAttribute('aria-pressed')") == "true")
        page.click("#htRestartBtn"); page.wait_for_timeout(200)
        check("controls/restartResetsRun",
              page.evaluate("({s: HT.screen, m: HT.mission, sc: HT.score})") ==
              {"s": "intro", "m": 0, "sc": 0})

        # Fullscreen button label round-trip
        page.evaluate("document.querySelector('#htPanel').requestFullscreen()")
        page.wait_for_timeout(700)
        check("controls/fullscreenLabelFlips",
              page.eval_on_selector("#htFsBtn", "e => e.textContent") == "Exit Fullscreen")
        page.evaluate("""(() => {
          const orig = window.ftSessionLeave;
          window.ftSessionLeave = () => {};
          document.exitFullscreen().finally(() => {
            setTimeout(() => { window.ftSessionLeave = orig; }, 500);
          });
        })()""")
        page.wait_for_timeout(900)
        check("controls/fullscreenLabelRestores",
              page.eval_on_selector("#htFsBtn", "e => e.textContent") == "Fullscreen")

        # Back to games
        page.click("#htExit"); page.wait_for_timeout(400)
        check("controls/backToGamesNavigates",
              page.evaluate("location.hash") == "#funtime" and
              "ft-playing" not in page.evaluate("document.body.className"))

        # ---------- HINTS / TIMER / ERRORS ----------
        enter_game(page)
        cmd(page, "start"); cmd(page, "begin")
        cmd(page, "hint")
        check("gameplay/hintHalvesReward",
              page.evaluate("HT.hintPenalty") == 0.5 and
              page.evaluate("HT.hintsUsed") == 1)
        cmd(page, "unlock 0000")
        check("gameplay/wrongAnswerCountsAttempt",
              page.evaluate("HT.attempts") == 1)
        page.evaluate("HT.remaining = 10; htTimerPaint()")
        check("gameplay/timerCritState",
              "crit" in page.eval_on_selector("#htTimer", "e => e.className"))
        page.evaluate("HT.remaining = 30; htTimerPaint()")
        check("gameplay/timerWarnState",
              "warn" in page.eval_on_selector("#htTimer", "e => e.className"))
        cmd(page, "clear")
        check("gameplay/clearWipesLog",
              page.evaluate("document.querySelector('#htOut').children.length") == 1)
        # During an ACTIVE mission any non-command input is treated as an
        # answer attempt (by design); the unknown-command report belongs to
        # the non-active screens. Verify both paths behave as designed.
        cmd(page, "definitely-not-a-command")
        check("gameplay/activeScreenTreatsInputAsAnswer",
              page.evaluate("HT.attempts") == 2,
              f"attempts={page.evaluate('HT.attempts')}")
        cmd(page, "restart"); page.wait_for_timeout(200)
        cmd(page, "definitely-not-a-command")
        unknown = page.evaluate(
            "[...document.querySelectorAll('#htOut .l-err')]"
            ".some(e => e.textContent.includes('Unknown command'))")
        check("gameplay/unknownCommandReportsOutsideMission", unknown)

        check("runtime/noPageErrors", not errors, "; ".join(errors[:3]))
        browser.close()

    failed = [r for r in RESULTS if not r[1]]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} checks passed")
    if failed:
        print("FAILED:")
        for name, _, detail in failed:
            print(f"  - {name}  [{detail}]")
        sys.exit(1)


if __name__ == "__main__":
    main()
