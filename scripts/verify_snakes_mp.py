#!/usr/bin/env python3
"""Snakes & Ladders LOCAL MULTIPLAYER verification: 2P / 3P / 4P on one
device, in real Chromium via Playwright. Mirrors scripts/verify_snakes.py.

Covers, independently per mode:
  • mode selector buttons actually clickable + correct player count/kinds
  • player name inputs: typing names flows into chips/log/turn indicator
  • dice rolling, turn rotation order, token isolation (only mover moves)
  • snakes & ladders per player, overshoot, exact win, winner overlay text
  • shared dice, active-player indicator, individual move history
  • restart (incl. mid-animation), fullscreen, resize, rapid-click races
"""
import json, math, random, sys, time
from playwright.sync_api import sync_playwright

BASE = "http://127.0.0.1:8912"
RESULTS = []

def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  [{detail}]" if detail else ""))

def make_force(page):
    """Retry a forced roll until the flow accepts it (SND.force refuses
    while another roll is animating — same rule as the Roll button)."""
    import time as T
    def force(v):
        dl = T.time() + 5
        while T.time() < dl:
            if page.evaluate(f"window.SND.force({v})"):
                return True
            page.wait_for_timeout(50)
        return False
    return force

with sync_playwright() as p:
    browser = p.chromium.launch()
    page = browser.new_page(viewport={"width": 1360, "height": 950})
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)

    page.goto(BASE + "/console")
    page.wait_for_selector("#viewRoot")
    force = make_force(page)

    # =====================================================================
    # SHARED SETUP: enter the game once, shrink timings for flow checks
    # =====================================================================
    page.evaluate("location.hash = '#funtime/sn'")
    page.wait_for_selector("#snBoard")
    page.wait_for_timeout(400)
    page.evaluate("window.SND.fast()")

    # ---------- setup panel reachable from the CARD-CLICK (fullscreen) path ----------
    page.evaluate("location.hash = '#funtime'")
    page.wait_for_timeout(400)
    page.click("#ftCardSn"); page.wait_for_timeout(900)
    fs_setup = page.evaluate("""(() => {
      const seg = document.getElementById('snModeSeg');
      const r = seg.getBoundingClientRect();
      return {fs: !!document.fullscreenElement, h: Math.round(r.height),
              visible: r.height > 0 && r.width > 0,
              clickable: document.elementFromPoint(r.left + r.width/2, r.top + 10) !== null}})()""")
    check("setup/visibleInFullscreenCardPath", fs_setup["fs"] and fs_setup["visible"], json.dumps(fs_setup))
    page.keyboard.press("Escape"); page.wait_for_timeout(600)

    # =====================================================================
    # PER-MODE SUITES (buttons clicked for real, names typed for real)
    # =====================================================================
    MODES = [
        ("2p", 2, ["Asha", "Bilal"]),
        ("3p", 3, ["Asha", "Bilal", "Chen"]),
        ("4p", 4, ["Asha", "Bilal", "Chen", "Divya"]),
    ]

    for mode, n, names in MODES:
        tag = f"{mode}/{n}p"
        page.evaluate("location.hash = '#funtime/sn'")
        page.wait_for_selector("#snBoard"); page.wait_for_timeout(300)

        # ---- select the mode by CLICKING the real button ----
        page.click(f"#snModeSeg button[data-v='{mode}']")
        page.wait_for_timeout(250)
        st = page.evaluate("""(m => ({
          on: document.querySelector(`#snModeSeg button[data-v='${m}']`).classList.contains('on'),
          chip: document.getElementById('snModeChip').textContent,
          n: window.SNG.game.players.length,
          kinds: window.SNG.game.players.map(p => p.kind)}))""", mode)
        check(f"{tag}/modeButtonSelects", st["on"] and st["n"] == n
              and all(k == "human" for k in st["kinds"]), json.dumps(st))

        # ---- name inputs: exactly n, all editable ----
        inputs = page.evaluate("""(() => {
          const ins = [...document.querySelectorAll('#snNames .sn-name-input')];
          return {count: ins.length, editable: ins.every(i => !i.disabled && !i.readOnly)};})()""")
        check(f"{tag}/nameInputsPresent", inputs["count"] == n and inputs["editable"], json.dumps(inputs))

        # ---- type the names for real ----
        for i, nm in enumerate(names):
            page.fill(f"#snName{i}", nm)
        page.wait_for_timeout(150)
        got = page.evaluate("window.SNG.names.slice()")
        check(f"{tag}/namesCaptured", got[:n] == names, json.dumps(got))

        # ---- Start Game commits the named players ----
        page.click("#snStart"); page.wait_for_timeout(250)
        st = page.evaluate("""(() => ({n: window.SNG.game.players.length,
          chips: [...document.querySelectorAll('#snPlayers .sn-pl .nm')].map(e => e.textContent.trim()),
          msg: document.getElementById('snMsg').textContent}))()""")
        chip_ok = all(nm in c for nm, c in zip(names, st["chips"]))
        check(f"{tag}/startNamedGame", st["n"] == n and chip_ok and all(nm in st["msg"] for nm in names[:1]),
              json.dumps({"chips": st["chips"], "msg": st["msg"][:60]}))

        # ---- turn rotation order over n*2 rolls ----
        order = []
        iso_ok = True
        for _ in range(n * 2):
            s = page.evaluate("({t: window.SNG.game.turn, phase: window.SNG.phase})")
            if s["phase"] != "idle":
                page.wait_for_timeout(120); continue
            before = page.evaluate("window.SNG.game.players.map(p => p.pos)")
            t = s["t"]; order.append(t)
            force((t % 6) + 1)
            page.wait_for_timeout(300)
            after = page.evaluate("window.SNG.game.players.map(p => p.pos)")
            if not (after[t] != before[t] and all(after[j] == before[j] for j in range(n) if j != t)):
                iso_ok = False
        want_order = [i % n for i in range(len(order))]
        check(f"{tag}/rotationOrder", order == want_order, f"{order}")
        check(f"{tag}/tokenIsolation", iso_ok)

        # ---- active-player indicator follows the turn ----
        for _ in range(n + 1):
            s = page.evaluate("({t: window.SNG.game.turn, phase: window.SNG.phase})")
            if s["phase"] == "idle":
                force(3)
            page.wait_for_timeout(220)
        # chip text uses custom names; verify both agree with engine turn
        agree = page.evaluate("""(() => {
          const g = window.SNG.game;
          const chip = document.getElementById('snTurnChip').querySelector('.v').textContent;
          const marked = document.querySelector('#snPlayers .sn-pl.turn');
          return {engineTurn: g.turn,
                  markedIdx: marked ? Number(marked.dataset.p) : -1,
                  chipName: chip};})()""")
        check(f"{tag}/activeIndicatorAgrees", agree["engineTurn"] == agree["markedIdx"],
              json.dumps(agree))

        # ---- roll button enabled for every human turn ----
        en = []
        for _ in range(n + 1):
            s = page.evaluate("({t: window.SNG.game.turn, phase: window.SNG.phase})")
            if s["phase"] == "idle":
                en.append(page.evaluate("!document.getElementById('snRollBtn').disabled"))
                force(2)
            page.wait_for_timeout(200)
        check(f"{tag}/rollBtnEveryHumanTurn", all(en) and len(en) >= n, f"{len(en)} samples")

        # ---- snake + ladder per a distinct player ----
        # pick player n-1, teleport near a snake head, force the landing
        page.evaluate(f"window.SNG.game.players[{n-1}].pos = 46")   # 46+1=47 snake -> 26
        page.evaluate(f"window.SNG.game.turn = {n-1}")
        force(1)
        page.wait_for_timeout(600)
        st = page.evaluate(f"window.SNG.game.players[{n-1}].pos")
        check(f"{tag}/snakeSlide", st == 26, str(st))
        page.evaluate(f"window.SNG.game.players[0].pos = 3")       # 3+1=4 ladder -> 14
        page.evaluate("window.SNG.game.turn = 0")
        force(1)
        page.wait_for_timeout(600)
        st = page.evaluate("window.SNG.game.players[0].pos")
        check(f"{tag}/ladderClimb", st == 14, str(st))

        # ---- individual move history: every log line attributed to its player ----
        hist = page.evaluate("""(() => {
          const g = window.SNG.game;
          const lines = [...document.querySelectorAll('#snLog div')];
          return {entries: g.history.length,
                  byPlayer: g.history.reduce((a,h)=>{a[h.player]=(a[h.player]||0)+1;return a;},{}),
                  domLines: lines.length,
                  namesInDom: lines.map(l => l.textContent).slice(0,3)};})()""")
        check(f"{tag}/individualHistory", hist["entries"] >= n * 2 and len(hist["byPlayer"]) >= min(n, 2),
              json.dumps({"entries": hist["entries"], "byPlayer": hist["byPlayer"]}))

        # ---- log shows custom names ----
        named_log = page.evaluate("""(() => {
          const t = [...document.querySelectorAll('#snLog div')].map(e=>e.textContent).join(' ');
          return {asha: t.includes('Asha'), bilal: t.includes('Bilal')};})()""")
        check(f"{tag}/logUsesCustomNames", named_log["asha"] or named_log["bilal"], json.dumps(named_log))

        # ---- overshoot wastes the turn ----
        page.evaluate(f"window.SNG.game.players[{n-1}].pos = 98")
        page.evaluate(f"window.SNG.game.turn = {n-1}")
        force(4)
        page.wait_for_timeout(500)
        st = page.evaluate(f"({{pos: window.SNG.game.players[{n-1}].pos, turn: window.SNG.game.turn}})")
        check(f"{tag}/overshoot", st["pos"] == 98 and st["turn"] == (n - 1 + 1) % n, json.dumps(st))

        # ---- exact win by a NAMED player: overlay must name them ----
        page.evaluate("window.SNG.game.players[1].pos = 96")
        page.evaluate("window.SNG.game.turn = 1")
        force(4)     # 96+4 = 100 exact
        page.wait_for_timeout(900)
        st = page.evaluate("""(() => ({
      winner: window.SNG.game.winner,
      overlay: !document.getElementById('snOverlay').classList.contains('hidden'),
      title: document.getElementById('snOvTitle').textContent,
      sub: document.getElementById('snOvSub').textContent}))()""")
        check(f"{tag}/exactWinNamesWinner", st["winner"] == 1 and st["overlay"]
              and names[1] in st["title"], json.dumps(st))
        check(f"{tag}/winOverlayMultiplayerWording",
              "you" not in st["sub"].lower() or names[1] in st["sub"], st["sub"])

        # ---- restart preserves names ----
        page.click("#snRestartBtn"); page.wait_for_timeout(300)
        st = page.evaluate("""(() => ({rolls: window.SNG.game.rolls, winner: window.SNG.game.winner,
      names: window.SNG.names.slice(0,4), n: window.SNG.game.players.length,
      overlayHidden: document.getElementById('snOverlay').classList.contains('hidden')}))()""")
        check(f"{tag}/restartPreservesNames", st["rolls"] == 0 and st["winner"] is None
              and st["n"] == n and st["names"][:n] == names
              and st["overlayHidden"], json.dumps(st))

        # ---- full random game completes; winner is on 100 ----
        deadline = time.time() + 90
        won = False; snap = {}
        while time.time() < deadline:
            snap = page.evaluate("({w: window.SNG.game.winner, phase: window.SNG.phase})")
            if snap["w"] is not None: won = True; break
            if snap["phase"] == "idle":
                force(math.floor(random.random()*6)+1)
            page.wait_for_timeout(40)
        check(f"{tag}/fullGameCompletes", won, f"winner={snap.get('w')}")
        if won:
            wpos = page.evaluate("window.SNG.game.players[window.SNG.game.winner].pos")
            check(f"{tag}/winnerOn100", wpos == 100, str(wpos))
            others = page.evaluate("""(() => {
      const g = window.SNG.game;
      return g.players.map((p,i)=>({i, pos:p.pos, on100: p.pos===100}));})()""")
            check(f"{tag}/singleWinner", sum(1 for o in others if o["on100"]) == 1, json.dumps(others))

    # =====================================================================
    # PVC REGRESSION (must be untouched)
    # =====================================================================
    page.evaluate("location.hash = '#funtime/sn'")
    page.wait_for_selector("#snBoard"); page.wait_for_timeout(300)
    page.click("#snModeSeg button[data-v='pvc']")
    page.wait_for_timeout(250)
    st = page.evaluate("""(() => ({
      n: window.SNG.game.players.length,
      kinds: window.SNG.game.players.map(p=>p.kind),
      nameRows: document.querySelectorAll('#snNames .sn-name-input').length,
      aiRow: document.querySelectorAll('#snNames .sn-name-row.ai').length,
      chip: document.getElementById('snModeChip').textContent}))()""")
    check("pvc/regressionConfig", st["n"] == 2 and st["kinds"] == ["human", "ai"]
          and st["nameRows"] == 1 and st["aiRow"] == 1, json.dumps(st))
    # AI still acts autonomously
    deadline = time.time() + 25
    ai_done = False
    while time.time() < deadline:
        s = page.evaluate("({t: window.SNG.game.turn, rolls: window.SNG.game.rolls, w: window.SNG.game.winner, phase: window.SNG.phase})")
        if s["w"] is not None:
            break
        if s["t"] == 1 and s["rolls"] > 0 and s["phase"] == "idle":
            ai_done = True; break          # AI rolled AND the flow settled
        if s["t"] == 0 and s["phase"] == "idle":
            force(math.floor(random.random()*6)+1)
        page.wait_for_timeout(120)
    check("pvc/aiAutonomous", ai_done, json.dumps(s))
    # PvC win overlay shows the winner's name prominently
    # (settle any in-flight flow first: a mode switch mid-animation would
    #  otherwise reject the forced roll and leave a STALE overlay visible)
    dl = time.time() + 5
    while time.time() < dl:
        if page.evaluate("window.SNG.phase") == "idle":
            break
        page.wait_for_timeout(50)
    page.evaluate("window.SNG.game.players[0].pos = 97")
    page.evaluate("window.SNG.game.turn = 0")
    assert force(3), "winning roll was rejected"
    page.wait_for_timeout(900)
    st = page.evaluate("""(() => ({
      w: window.SNG.game.winner,
      name: (document.querySelector('#snOvTitle .sn-ov-name')||{}).textContent || null,
      crown: !!document.querySelector('#snOvTitle .sn-ov-crown'),
      confetti: !!window.SNCF.raf}))()""")
    # Task 3 spec: the winner's NAME is shown prominently (with crown +
    # confetti), replacing the old generic "Victory!" wording.
    check("pvc/soloWinShowsWinnerName", st["w"] == 0 and st["name"]
          and st["crown"] and st["confetti"], json.dumps(st))

    # 2VC regression: two humans + one AI
    page.click("#snModeSeg button[data-v='2vc']")
    page.wait_for_timeout(250)
    st = page.evaluate("""(() => ({
      n: window.SNG.game.players.length,
      kinds: window.SNG.game.players.map(p=>p.kind),
      nameRows: document.querySelectorAll('#snNames .sn-name-input').length,
      aiRows: document.querySelectorAll('#snNames .sn-name-row.ai').length}))()""")
    check("2vc/regressionConfig", st["n"] == 3 and st["kinds"] == ["human","human","ai"]
          and st["nameRows"] == 2 and st["aiRows"] == 1, json.dumps(st))

    # =====================================================================
    # RAPID INTERACTION + ANIMATION RACES (4P)
    # =====================================================================
    page.click("#snModeSeg button[data-v='4p']")
    page.wait_for_timeout(250)
    # rapid roll clicks
    for _ in range(15):
        page.click("#snRollBtn", force=True)
        page.wait_for_timeout(30)
    page.wait_for_timeout(700)
    st = page.evaluate("""(() => ({
      rolls: window.SNG.game.rolls, phase: window.SNG.phase,
      sane: window.SNG.game.players.every(p => p.pos >= 0 && p.pos <= 100),
      winner: window.SNG.game.winner}))()""")
    check("race/rapidClicksSafe", st["sane"] and st["rolls"] <= 15, json.dumps(st))

    # restart DURING walk animation
    force(6)
    page.wait_for_timeout(60)                    # mid-walk
    page.click("#snRestartBtn")
    page.wait_for_timeout(300)
    st = page.evaluate("""(() => ({rolls: window.SNG.game.rolls, anim: window.SNG.anim,
      viz: window.SNG.viz, phase: window.SNG.phase,
      sane: window.SNG.game.players.every(p => p.pos === 0)}))()""")
    check("race/restartMidAnimation", st["rolls"] == 0 and st["anim"] is None
          and st["viz"] is None and st["sane"], json.dumps(st))

    # mode switch DURING animation must not corrupt state
    force(5)
    page.wait_for_timeout(60)
    page.click("#snModeSeg button[data-v='2p']")
    page.wait_for_timeout(400)
    st = page.evaluate("""(() => ({n: window.SNG.game.players.length,
      rolls: window.SNG.game.rolls, phase: window.SNG.phase,
      sane: window.SNG.game.players.every(p => p.pos >= 0 && p.pos <= 100)}))()""")
    check("race/modeSwitchMidAnimation", st["n"] == 2 and st["rolls"] == 0
          and st["phase"] in ("idle", "rolling", "moving") and st["sane"], json.dumps(st))

    # name typing DURING an animation must not disturb the flow
    force(4)
    page.wait_for_timeout(40)
    page.fill("#snName0", "Zoe")
    page.wait_for_timeout(500)
    st = page.evaluate("""(() => ({n: window.SNG.game.players.length,
      name0: window.SNG.names[0], rolls: window.SNG.game.rolls,
      sane: window.SNG.game.players.every(p => p.pos >= 0 && p.pos <= 100)}))()""")
    check("race/nameTypingDuringAnim", st["name0"] == "Zoe" and st["sane"], json.dumps(st))

    # =====================================================================
    # FULLSCREEN + RESIZE with 4P + names
    # =====================================================================
    page.click("#snModeSeg button[data-v='4p']")
    page.wait_for_timeout(200)
    for i, nm in enumerate(["Asha", "Bilal", "Chen", "Divya"]):
        page.fill(f"#snName{i}", nm)
    page.click("#snStart"); page.wait_for_timeout(300)
    force(3); page.wait_for_timeout(600)

    page.click("#snFsBtn"); page.wait_for_timeout(800)
    fs = page.evaluate("""(() => {
      const c = document.getElementById('snBoard');
      const r = c.getBoundingClientRect();
      const seg = document.getElementById('snModeSeg').getBoundingClientRect();
      return {native: !!document.fullscreenElement, w: Math.round(r.width),
              inVp: r.top >= -1 && r.bottom <= innerHeight + 1,
              segVisible: seg.height > 0};})()""")
    check("fullscreen/4pBoardFits", fs["native"] and fs["inVp"] and fs["w"] > 300, json.dumps(fs))
    check("fullscreen/setupReachable", fs["segVisible"])
    page.screenshot(path="/tmp/sn-shots/mp-4p-fullscreen.png")

    # resize: Chromium can't resize a fullscreen window; Esc also exits the
    # game (shared session behaviour), so re-enter via hash before resizing.
    page.keyboard.press("Escape"); page.wait_for_timeout(600)
    page.evaluate("location.hash = '#funtime/sn'")
    page.wait_for_selector("#snBoard"); page.wait_for_timeout(400)
    page.set_viewport_size({"width": 1280, "height": 720})
    page.wait_for_timeout(400)
    inline2 = page.evaluate("""(() => {
      const r = document.getElementById('snBoard').getBoundingClientRect();
      return {w: Math.round(r.width), inVp: r.top >= -1 && r.bottom <= innerHeight + 1};})()""")
    check("fullscreen/inlineRefitsAfterResize", inline2["inVp"], json.dumps(inline2))
    page.click("#snFsBtn"); page.wait_for_timeout(800)
    fs2 = page.evaluate("""(() => {
      const r = document.getElementById('snBoard').getBoundingClientRect();
      return {native: !!document.fullscreenElement, w: Math.round(r.width),
              inVp: r.top >= -1 && r.bottom <= innerHeight + 1};})()""")
    check("fullscreen/resizeRefits", fs2["native"] and fs2["inVp"], json.dumps(fs2))

    page.keyboard.press("Escape"); page.wait_for_timeout(700)

    # responsive sweep WITHOUT fullscreen (inline play), 4P with names
    for w, h in [(390, 844), (768, 1024), (1920, 1080)]:
        page.set_viewport_size({"width": w, "height": h})
        page.evaluate("location.hash = '#funtime/sn'")
        page.wait_for_selector("#snBoard"); page.wait_for_timeout(400)
        b = page.evaluate("""(() => {
          const r = document.getElementById('snBoard').getBoundingClientRect();
          const names = document.getElementById('snNames').getBoundingClientRect();
          return {w: Math.round(r.width), inVp: r.top >= -1 && r.left >= -1
                  && r.right <= innerWidth + 1 && r.bottom <= innerHeight + 1,
                  namesVisible: names.height > 0};})()""")
        check(f"responsive/{w}x{h}-4p", b["inVp"] and b["w"] >= 280, json.dumps(b))

    # =====================================================================
    # NAV INTEGRITY: all 10 cards still present
    # =====================================================================
    page.set_viewport_size({"width": 1360, "height": 950})
    page.evaluate("location.hash = '#funtime'")
    page.wait_for_timeout(500)
    cards = page.evaluate("document.querySelectorAll('.ft-card').length")
    check("nav/allCardsIntact", cards >= 10, f"{cards} cards")
    for cid in ["ftCardT3","ftCardSi","ftCardHt","ftCardCr","ftCardCh","ftCardTm","ftCardSo","ftCardFn","ftCardLu","ftCardSn"]:
        check(f"nav/{cid}", page.evaluate(f"!!document.getElementById('{cid}')"))

    # =====================================================================
    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))
    browser.close()

fails = [r for r in RESULTS if not r[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
