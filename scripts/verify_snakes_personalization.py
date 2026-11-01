#!/usr/bin/env python3
"""Snakes & Ladders PERSONALIZATION + PREMIUM verification: custom names in
every mode, name display everywhere, winner celebration with confetti,
per-player match statistics, and rematch semantics — in real Chromium via
Playwright. Mirrors scripts/verify_snakes.py conventions.

Scenarios (per the task list):
 1. Two-player local game          6. Winner announcement
 2. Three-player local game        7. Match statistics
 3. Four-player local game         8. Rematch
 4. Player vs Computer             9. Restart and fullscreen
 5. Custom names in every mode
"""
import json, math, random, sys, time
from playwright.sync_api import sync_playwright

BASE = "http://127.0.0.1:8912"
RESULTS = []

def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  [{detail}]" if detail else ""))

with sync_playwright() as p:
    browser = p.chromium.launch()
    page = browser.new_page(viewport={"width": 1360, "height": 950})
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)

    page.goto(BASE + "/console")
    page.wait_for_selector("#viewRoot")
    page.evaluate("location.hash = '#funtime/sn'")
    page.wait_for_selector("#snBoard")
    page.wait_for_timeout(400)
    page.evaluate("window.SND.fast()")

    def wait_idle(tmo=5):
        dl = time.time() + tmo
        while time.time() < dl:
            if page.evaluate("window.SNG.phase") == "idle":
                return True
            page.wait_for_timeout(50)
        return False

    def force_roll(v):
        """Force a roll, retrying until the flow accepts it (the hook now
        refuses while another roll is animating — same rule as the button)."""
        dl = time.time() + 5
        while time.time() < dl:
            if page.evaluate(f"window.SND.force({v})"):
                return True
            page.wait_for_timeout(50)
        return False

    def play(seq):
        """Force a deterministic roll sequence; asserts turn order first."""
        for who, v in seq:
            assert wait_idle(), "previous roll never settled"
            t = page.evaluate("window.SNG.game.turn")
            assert t == who, f"turn mismatch: expected {who} got {t}"
            assert force_roll(v), "forced roll rejected"
            assert wait_idle(), "roll never resolved"
            page.wait_for_timeout(60)

    NAMES4 = ["Himanshu", "Rahul", "Aman", "Priya"]

    # =====================================================================
    # SCENARIOS 1-3 + 5: 2P / 3P / 4P with custom names
    # =====================================================================
    for mode, n, names in [("2p", 2, NAMES4[:2]), ("3p", 3, NAMES4[:3]), ("4p", 4, NAMES4)]:
        tag = f"{mode}/{n}p"
        page.click(f"#snModeSeg button[data-v='{mode}']")
        page.wait_for_timeout(250)
        st = page.evaluate("""(m => ({
          on: document.querySelector(`#snModeSeg button[data-v='${m}']`).classList.contains('on'),
          n: window.SNG.game.players.length,
          kinds: window.SNG.game.players.map(p=>p.kind)}))""", mode)
        check(f"{tag}/config", st["on"] and st["n"] == n and all(k == "human" for k in st["kinds"]), json.dumps(st))

        for i, nm in enumerate(names):
            page.fill(f"#snName{i}", nm)
        page.click("#snStart"); page.wait_for_timeout(250); wait_idle()

        # ---- Task 1: names captured & preserved ----
        got = page.evaluate("window.SNG.names.slice()")
        check(f"{tag}/namesCaptured", got[:n] == names, json.dumps(got))

        # ---- Task 2: top turn indicator shows possessive name ----
        chip = page.evaluate("document.getElementById('snTurnChip').querySelector('.v').textContent")
        check(f"{tag}/turnChipPossessive", chip == f"{names[0]}'s Turn", chip)

        # ---- Task 2: player cards show names ----
        chips = page.evaluate("[...document.querySelectorAll('#snPlayers .sn-pl .nm-t')].map(e=>e.textContent.trim())")
        check(f"{tag}/playerCardsNamed", chips == names, json.dumps(chips))

        # ---- Task 2: move history uses names ("X rolled N") ----
        force_roll(3)
        wait_idle(); page.wait_for_timeout(80)
        log = page.evaluate("[...document.querySelectorAll('#snLog div')].slice(0,2).map(e=>e.textContent)")
        check(f"{tag}/logUsesNames", any(names[i] in l and "rolled" in l for i, l in enumerate(log)), json.dumps(log))

        # ---- Task 2: snake message names the victim ("got bitten by a snake") ----
        page.evaluate(f"window.SNG.game.players[0].pos = 46")
        page.evaluate("window.SNG.game.turn = 0")
        force_roll(1)     # 47 -> snake -> 26
        wait_idle(); page.wait_for_timeout(80)
        log = page.evaluate("[...document.querySelectorAll('#snLog div')].slice(0,3).map(e=>e.textContent)")
        ok = any(names[0] in l and "snake" in l.lower() for l in log)
        check(f"{tag}/snakeMessageNamed", ok, json.dumps(log))

        # ---- Task 2: ladder message names the climber ----
        p1 = 1 % n
        page.evaluate(f"window.SNG.game.players[{p1}].pos = 8")
        page.evaluate(f"window.SNG.game.turn = {p1}")
        force_roll(1)     # 9 -> ladder -> 31
        wait_idle(); page.wait_for_timeout(80)
        log = page.evaluate("[...document.querySelectorAll('#snLog div')].slice(0,3).map(e=>e.textContent)")
        ok = any(names[p1] in l and "ladder" in l.lower() for l in log)
        check(f"{tag}/ladderMessageNamed", ok, json.dumps(log))

        # ---- Task 4: live per-player stats accumulate correctly ----
        st = page.evaluate("window.SNG.stats")
        s0 = st[0]
        check(f"{tag}/statsAccumulate", s0["rolls"] >= 2 and s0["moves"] >= 2 and s0["snakes"] == 1,
              json.dumps(s0))

        # ---- Task 3+6: winner celebration ----
        page.evaluate(f"window.SNG.game.players[{p1}].pos = 96")
        page.evaluate(f"window.SNG.game.turn = {p1}")
        wait_idle()
        assert force_roll(4), "winning roll rejected"   # exact 100
        page.wait_for_timeout(1000)
        ov = page.evaluate("""(() => ({
          shown: !document.getElementById('snOverlay').classList.contains('hidden'),
          winner: window.SNG.game.winner,
          name: (document.querySelector('#snOvTitle .sn-ov-name')||{}).textContent || null,
          crown: !!document.querySelector('#snOvTitle .sn-ov-crown'),
          popAnim: (document.querySelector('#snOvTitle .sn-ov-winner')||{}) &&
                   getComputedStyle(document.querySelector('#snOvTitle .sn-ov-winner')).animationName,
          sub: document.getElementById('snOvSub').textContent,
          statRow: [...document.querySelectorAll('#snOvStats .sn-ov-stat')].map(e=>e.querySelector('.v').textContent),
          tableRows: document.querySelectorAll('#snOvTable tbody tr').length,
          buttons: [...document.querySelectorAll('#snOvRow .sn-ov-btn')].map(e=>e.textContent)}))()""")
        wname = names[p1]
        check(f"{tag}/winnerAnnounced", ov["shown"] and ov["winner"] == p1
              and ov["name"] == wname, json.dumps({"winner": ov["winner"], "name": ov["name"]}))
        check(f"{tag}/celebrationCrownPop", ov["crown"] and ov["popAnim"] == "snPopIn",
              f"crown={ov['crown']} anim={ov['popAnim']}")
        check(f"{tag}/confettiRunning", page.evaluate("!!window.SNCF.raf"))
        cf_px = page.evaluate("""(() => {
          const cv = document.getElementById('snConfetti');
          if (!cv || !cv.width) return 0;
          const d = cv.getContext('2d').getImageData(0,0,cv.width,cv.height).data;
          let n=0; for (let i=3;i<d.length;i+=4*53){ if(d[i]>0) n++; }
          return n; })()""")
        check(f"{tag}/confettiDrawsPixels", cf_px > 20, f"{cf_px} sampled px")
        check(f"{tag}/overlayStatTotals", len(ov["statRow"]) == 3, json.dumps(ov["statRow"]))
        check(f"{tag}/statsTablePerPlayer", ov["tableRows"] == n
              and ov["buttons"] == ["Play Again", "Back to Games"],
              f"rows={ov['tableRows']} btns={ov['buttons']}")

        # ---- Task 4: stats table attribution matches SNG.stats ----
        tbl = page.evaluate("""(() => [...document.querySelectorAll('#snOvTable tbody tr')].map(tr =>
          [...tr.querySelectorAll('td')].map(td=>td.textContent.trim())) )()""")
        eng = page.evaluate("window.SNG.stats.map(s=>[String(s.rolls),String(s.moves),String(s.snakes),String(s.ladders),String(s.sixes)])")
        ok = all(tbl[i][1:] == eng[i] for i in range(n))
        check(f"{tag}/statsTableMatchesEngine", ok, json.dumps({"tbl": tbl, "eng": eng}))

        # ---- Task 5: REMATCH keeps names/colors, resets everything else ----
        page.click("#snOvRow .sn-ov-btn.primary")   # Play Again
        page.wait_for_timeout(400)
        rm = page.evaluate("""(() => ({
          names: window.SNG.names.slice(0,4),
          chips: [...document.querySelectorAll('#snPlayers .sn-pl .nm-t')].map(e=>e.textContent.trim()),
          positions: window.SNG.game.players.map(p=>p.pos),
          rolls: window.SNG.game.rolls,
          turn: window.SNG.game.turn,
          winner: window.SNG.game.winner,
          statsZero: window.SNG.stats.every(s=>s.rolls===0&&s.moves===0&&s.snakes===0&&s.ladders===0&&s.sixes===0),
          overlayHidden: document.getElementById('snOverlay').classList.contains('hidden'),
          confettiStopped: !window.SNCF.raf}))()""")
        check(f"{tag}/rematchKeepsNames", rm["chips"] == names and rm["names"][:n] == names,
              json.dumps(rm["chips"]))
        check(f"{tag}/rematchResetsState", rm["positions"] == [0]*n and rm["rolls"] == 0
              and rm["turn"] == 0 and rm["winner"] is None and rm["statsZero"]
              and rm["overlayHidden"] and rm["confettiStopped"], json.dumps(rm))

        # colors preserved: token color array unchanged
        cols = page.evaluate("window.SnakesEngine.COLORS.slice()")
        check(f"{tag}/rematchKeepsColors", cols == ["red","green","yellow","blue"], json.dumps(cols))

        # ---- SCENARIO 9a: restart (button) mid-game also preserves names ----
        force_roll(4); wait_idle()
        page.click("#snRestartBtn"); page.wait_for_timeout(350)
        rs = page.evaluate("""(() => ({
          rolls: window.SNG.game.rolls, pos: window.SNG.game.players.map(p=>p.pos),
          chips: [...document.querySelectorAll('#snPlayers .sn-pl .nm-t')].map(e=>e.textContent.trim()),
          statsZero: window.SNG.stats.every(s=>s.rolls===0)}))()""")
        check(f"{tag}/restartPreservesNames", rs["chips"] == names and rs["rolls"] == 0
              and rs["pos"] == [0]*n, json.dumps(rs["chips"]))

    # =====================================================================
    # SCENARIO 4: Player vs Computer (custom name + AI default name)
    # =====================================================================
    page.click("#snModeSeg button[data-v='pvc']"); page.wait_for_timeout(250)
    ai_row = page.evaluate("document.querySelector('#snNames .sn-name-row.ai .sn-name-static').textContent")
    check("pvc/aiDefaultName", ai_row in ("Nova", "Orion", "Vega", "Pixel"), ai_row)
    n_inputs = page.evaluate("document.querySelectorAll('#snNames .sn-name-input').length")
    check("pvc/oneHumanInput", n_inputs == 1, str(n_inputs))

    page.fill("#snName0", "Himanshu")
    page.click("#snStart"); page.wait_for_timeout(250)
    chips = page.evaluate("[...document.querySelectorAll('#snPlayers .sn-pl .nm-t')].map(e=>e.textContent.trim())")
    check("pvc/chipsShowHumanAndAI", chips[0] == "Himanshu" and chips[1] == ai_row, json.dumps(chips))

    # AI acts autonomously; its rolls get attributed in stats
    deadline = time.time() + 25
    ai_rolled = False
    while time.time() < deadline:
        s = page.evaluate("({t: window.SNG.game.turn, r: window.SNG.game.rolls, w: window.SNG.game.winner, ph: window.SNG.phase})")
        if s["w"] is not None:
            break
        if s["t"] == 1 and s["r"] > 0 and s["ph"] == "idle":
            ai_rolled = True; break          # AI rolled AND the flow settled
        if s["t"] == 0 and s["ph"] == "idle":
            force_roll(math.floor(random.random()*6)+1)
        page.wait_for_timeout(120)
    check("pvc/aiAutonomous", ai_rolled, json.dumps(s))

    # human win in PvC: solo wording + confetti
    wait_idle()
    page.evaluate("window.SNG.game.players[0].pos = 97; window.SNG.game.turn = 0")
    assert force_roll(3); page.wait_for_timeout(1000)
    ov = page.evaluate("""(() => ({
      w: window.SNG.game.winner,
      name: (document.querySelector('#snOvTitle .sn-ov-name')||{}).textContent,
      sub: document.getElementById('snOvSub').textContent,
      confetti: !!window.SNCF.raf}))()""")
    check("pvc/soloWinCelebration", ov["w"] == 0 and ov["name"] == "Himanshu" and ov["confetti"],
          json.dumps(ov))

    # AI win in PvC: still celebrates with the AI's name
    page.click("#snOvRow .sn-ov-btn.primary"); page.wait_for_timeout(300)
    wait_idle()
    page.evaluate("window.SNG.game.players[1].pos = 99; window.SNG.game.turn = 1")
    assert force_roll(1); page.wait_for_timeout(1000)
    ov = page.evaluate("""(() => ({
      w: window.SNG.game.winner,
      name: (document.querySelector('#snOvTitle .sn-ov-name')||{}).textContent,
      sub: document.getElementById('snOvSub').textContent,
      confetti: !!window.SNCF.raf}))()""")
    check("pvc/aiWinCelebration", ov["w"] == 1 and ov["name"] == ai_row and ov["confetti"],
          json.dumps(ov))

    # =====================================================================
    # TASK 1 edge cases: empty names, duplicates
    # =====================================================================
    page.click("#snOvRow .sn-ov-btn.primary"); page.wait_for_timeout(300)
    page.click("#snModeSeg button[data-v='4p']"); page.wait_for_timeout(250)
    # duplicates flagged visually...
    page.fill("#snName0", "Rahul"); page.fill("#snName1", "Rahul"); page.wait_for_timeout(200)
    dup = page.evaluate("""(() => ({
      flagged: document.getElementById('snName1').classList.contains('dup'),
      title: document.getElementById('snName1').title}))()""")
    check("names/duplicateFlagged", dup["flagged"], json.dumps(dup))
    # ...and handled gracefully at play time
    page.click("#snStart"); page.wait_for_timeout(300)
    chips = page.evaluate("[...document.querySelectorAll('#snPlayers .sn-pl .nm-t')].map(e=>e.textContent.trim())")
    uniq = len(set(chips)) == 4
    check("names/duplicatesResolvedUniquely", uniq and chips[1].startswith("Rahul"), json.dumps(chips))

    # empty names fall back to colour defaults
    page.fill("#snName2", "   "); page.fill("#snName3", ""); page.wait_for_timeout(150)
    page.click("#snStart"); page.wait_for_timeout(300)
    chips = page.evaluate("[...document.querySelectorAll('#snPlayers .sn-pl .nm-t')].map(e=>e.textContent.trim())")
    check("names/emptyFallsBackToColour", chips[2] == "Yellow" and chips[3] == "Blue", json.dumps(chips))

    # name survives mode switch away and back
    page.fill("#snName0", "Himanshu"); page.wait_for_timeout(100)
    page.click("#snModeSeg button[data-v='2p']"); page.wait_for_timeout(250)
    page.click("#snModeSeg button[data-v='4p']"); page.wait_for_timeout(250)
    v = page.evaluate("document.getElementById('snName0').value")
    check("names/surviveModeSwitch", v == "Himanshu", v)

    # =====================================================================
    # SCENARIO 9b: fullscreen + resize with 4 named players
    # =====================================================================
    for i, nm in enumerate(NAMES4):
        page.fill(f"#snName{i}", nm)
    page.click("#snStart"); page.wait_for_timeout(250)
    force_roll(4); wait_idle()

    page.click("#snFsBtn"); page.wait_for_timeout(900)
    fs = page.evaluate("""(() => {
      const r = document.getElementById('snBoard').getBoundingClientRect();
      return {native: !!document.fullscreenElement, w: Math.round(r.width),
              inVp: r.top >= -1 && r.bottom <= innerHeight + 1,
              chips: [...document.querySelectorAll('#snPlayers .sn-pl .nm-t')].map(e=>e.textContent.trim())};})()""")
    check("fullscreen/4pBoardFits", fs["native"] and fs["inVp"] and fs["w"] > 300, json.dumps(fs))
    check("fullscreen/namesStillShown", fs["chips"] == NAMES4, json.dumps(fs["chips"]))
    page.screenshot(path="/tmp/sn-shots/personalized-4p-fs.png")

    page.keyboard.press("Escape"); page.wait_for_timeout(600)
    page.evaluate("location.hash = '#funtime/sn'")
    page.wait_for_selector("#snBoard"); page.wait_for_timeout(400)
    for w, h in [(390, 844), (1920, 1080)]:
        page.set_viewport_size({"width": w, "height": h})
        page.wait_for_timeout(400)
        b = page.evaluate("""(() => {
          const r = document.getElementById('snBoard').getBoundingClientRect();
          return {w: Math.round(r.width), inVp: r.top >= -1 && r.left >= -1
                  && r.right <= innerWidth+1 && r.bottom <= innerHeight+1};})()""")
        check(f"responsive/{w}x{h}", b["inVp"] and b["w"] >= 280, json.dumps(b))

    # =====================================================================
    # RAPID INTERACTION SAFETY (regression guard)
    # =====================================================================
    page.set_viewport_size({"width": 1360, "height": 950})
    page.wait_for_timeout(300)
    for _ in range(12):
        page.click("#snRollBtn", force=True)
        page.wait_for_timeout(30)
    page.wait_for_timeout(700)
    st = page.evaluate("""(() => ({
      sane: window.SNG.game.players.every(p=>p.pos>=0&&p.pos<=100),
      phase: window.SNG.phase,
      statsLen: window.SNG.stats.length === window.SNG.game.players.length}))()""")
    check("race/rapidClicksSafe", st["sane"] and st["statsLen"], json.dumps(st))

    # restart DURING animation keeps stats consistent
    page.evaluate("window.SND.force(5)")   # may be rejected mid-anim: fine
    page.wait_for_timeout(50)
    page.click("#snRestartBtn"); page.wait_for_timeout(350)
    st = page.evaluate("""(() => ({rolls: window.SNG.game.rolls,
      statsZero: window.SNG.stats.every(s=>s.rolls===0),
      names: window.SNG.names.slice(0,4)}))()""")
    check("race/restartMidAnimation", st["rolls"] == 0 and st["statsZero"], json.dumps(st))

    # =====================================================================
    # NAV INTEGRITY
    # =====================================================================
    page.evaluate("location.hash = '#funtime'")
    page.wait_for_timeout(500)
    cards = page.evaluate("document.querySelectorAll('.ft-card').length")
    check("nav/allCardsIntact", cards >= 10, f"{cards} cards")

    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))
    browser.close()

fails = [r for r in RESULTS if not r[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
