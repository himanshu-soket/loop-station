#!/usr/bin/env python3
"""PHASE 10 QA: full gameplay verification of the redesigned UI.
Covers: 2P/3P/4P/PvC, dice rolling, snake/ladder/token movement, win,
restart, fullscreen — with screenshots at each stage. Real Chromium.
"""
import json, sys, time
from playwright.sync_api import sync_playwright

BASE = "http://127.0.0.1:8912"
OUT = "/tmp/sn-shots/after"
RESULTS = []

def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  [{detail}]" if detail else ""))

with sync_playwright() as p:
    b = p.chromium.launch()
    page = b.new_page(viewport={"width":1366,"height":768})
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
    page.goto(BASE + "/console"); page.wait_for_selector("#viewRoot")

    def enter():
        page.evaluate("location.hash = '#funtime/sn'")
        page.wait_for_selector("#snBoard"); page.wait_for_timeout(500)

    def wait_idle(tmo=5):
        dl = time.time() + tmo
        while time.time() < dl:
            if page.evaluate("window.SNG.phase") == "idle": return True
            page.wait_for_timeout(50)
        return False

    def force(v):
        dl = time.time() + 5
        while time.time() < dl:
            if page.evaluate(f"window.SND.force({v})"): return True
            page.wait_for_timeout(40)
        return False

    def wait_turn(i, tmo=8):
        """Wait until player i holds the turn AND the flow is idle."""
        dl = time.time() + tmo
        while time.time() < dl:
            s = page.evaluate("({t: window.SNG.game.turn, ph: window.SNG.phase, w: window.SNG.game.winner})")
            if s["w"] is not None: return False
            if s["t"] == i and s["ph"] == "idle": return True
            page.wait_for_timeout(60)
        return False

    def force_as(i, v):
        """Force value v as player i (waits for their turn first)."""
        return wait_turn(i) and force(v)

    enter()
    page.evaluate("window.SND.fast()")

    # ---------- every mode: config + a few rolls + token movement ----------
    for mode, names in [("pvc", ["Himanshu"]), ("2p", ["Himanshu","Rahul"]),
                        ("3p", ["Himanshu","Rahul","Priya"]),
                        ("4p", ["Himanshu","Rahul","Aman","Priya"])]:
        page.click(f"#snModeSeg button[data-v='{mode}']"); page.wait_for_timeout(250)
        for i, nm in enumerate(names):
            page.fill(f"#snName{i}", nm)
        page.click("#snStart"); page.wait_for_timeout(300); wait_idle()

        n = page.evaluate("window.SNG.game.players.length")
        st = page.evaluate("""(() => ({
          players: window.SNG.game.players.length,
          kinds: window.SNG.game.players.map(p=>p.kind),
          chips: [...document.querySelectorAll('#snPlayers .sn-pl .nm-t')].map(e=>e.textContent.trim())}))()""")
        exp_kinds = {"pvc":["human","ai"],"2p":["human","human"],
                     "3p":["human","human","human"],"4p":["human","human","human","human"]}[mode]
        ai_names = ["Nova", "Orion", "Vega", "Pixel"]
        exp_chips = [nm if k == "human" else ai_names[i]
                     for i, (nm, k) in enumerate(zip(names, exp_kinds))]
        # modes with more seats than provided names fill with humans/AI per registry
        if mode == "pvc": exp_chips = [names[0], "Orion"]
        check(f"{mode}/config", st["players"] == n and st["kinds"] == exp_kinds
              and st["chips"] == exp_chips, json.dumps({"got": st["chips"], "want": exp_chips}))

        # roll: dice widget animates + token moves + log line appears.
        # (the die face clears when the NEXT turn starts, so the durable
        #  evidence is the log line + the engine position)
        pos_before = page.evaluate("window.SNG.game.players.map(p=>p.pos)")
        assert force_as(0, 3) and wait_idle()
        st = page.evaluate("""((nm) => ({
          pos: window.SNG.game.players.map(p=>p.pos),
          logs: [...document.querySelectorAll('#snLog div')].slice(0,4).map(e=>e.textContent),
          rolls: window.SNG.stats[0].rolls}))()""", names[0])
        moved = st["pos"][0] != pos_before[0]
        # Himanshu's own line may already be buried under an AI follow-up roll
        his_line = next((l for l in st["logs"] if l.startswith(names[0])), "")
        check(f"{mode}/diceAndTokenMove",
              moved and st["rolls"] >= 1 and names[0] in his_line and "3" in his_line,
              json.dumps({"moved": moved, "rolls": st["rolls"], "line": his_line[:44]}))

        # snake slide: force player 0 onto 46 then roll 1 (47 -> snake).
        # In PvC an AI think-timer may still be pending; bumping the game
        # clock cancels it so the forced state can never be raced.
        page.evaluate("""(() => {
          window.SNG.clock++;                     // cancel pending AI timers
          const g = window.SNG.game;
          g.players[0].pos = 46; g.turn = 0;
          window.SNG.phase = 'idle';
        })()""")
        page.wait_for_timeout(120)                # let any cancelled timer die
        assert force_as(0, 1) and wait_idle()
        st = page.evaluate("""(() => ({
          pos: window.SNG.game.players[0].pos,
          log: [...document.querySelectorAll('#snLog div')].slice(0,2).map(e=>e.textContent).join(' | ')}))()""")
        check(f"{mode}/snakeSlide", st["pos"] < 47 and "snake" in st["log"].lower(),
              json.dumps(st))

        # ladder climb: player 1 onto 8, roll 1 (9 -> 31)
        if n >= 2:
            page.evaluate("""(() => {
              window.SNG.clock++;
              const g = window.SNG.game;
              g.players[1].pos = 8; g.turn = 1;
              window.SNG.phase = 'idle';
            })()""")
            page.wait_for_timeout(120)
            assert force_as(1, 1) and wait_idle()
            st = page.evaluate("window.SNG.game.players[1].pos")
            check(f"{mode}/ladderClimb", st > 9, f"landed on {st}")

        page.screenshot(path=f"{OUT}/qa-{mode}-playing.png")

        # win + celebration + restart
        page.evaluate("""(() => {
          window.SNG.clock++;
          const g = window.SNG.game;
          g.players[0].pos = 96; g.turn = 0;
          window.SNG.phase = 'idle';
        })()""")
        page.wait_for_timeout(120)
        assert force_as(0, 4); page.wait_for_timeout(900)
        st = page.evaluate("""(() => ({
      w: window.SNG.game.winner,
      name: (document.querySelector('#snOvTitle .sn-ov-name')||{}).textContent,
      crown: !!document.querySelector('#snOvTitle .sn-ov-crown'),
      confetti: !!window.SNCF.raf,
      rows: document.querySelectorAll('#snOvTable tbody tr').length}))()""")
        check(f"{mode}/winCelebration",
              st["w"] == 0 and st["name"] == names[0] and st["crown"]
              and st["confetti"] and st["rows"] == n, json.dumps(st))
        page.screenshot(path=f"{OUT}/qa-{mode}-winner.png")

        page.click("#snOvRow .sn-ov-btn.primary"); page.wait_for_timeout(400)
        st = page.evaluate("""(() => ({rolls: window.SNG.game.rolls,
      pos: window.SNG.game.players.map(p=>p.pos),
      names: window.SNG.names.slice(0,4)}))()""")
        check(f"{mode}/restartClean", st["rolls"] == 0 and all(x == 0 for x in st["pos"])
              and st["names"][:n] == names, json.dumps(st))

    # ---------- fullscreen + Esc ----------
    page.click("#snFsBtn"); page.wait_for_timeout(900)
    st = page.evaluate("""(() => ({
      native: !!document.fullscreenElement,
      w: Math.round(document.getElementById('snBoard').getBoundingClientRect().width)}))()""")
    check("fullscreen/entered", st["native"] and st["w"] > 400, json.dumps(st))
    page.screenshot(path=f"{OUT}/qa-fullscreen.png")
    page.keyboard.press("Escape"); page.wait_for_timeout(700)
    st = page.evaluate("""(() => ({native: !!document.fullscreenElement,
      back: !!document.getElementById('ftCardSn')}))()""")
    check("fullscreen/escExits", not st["native"] and st["back"])

    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))
    b.close()

fails = [r for r in RESULTS if not r[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
