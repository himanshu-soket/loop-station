#!/usr/bin/env python3
"""Snakes & Ladders functional verification: engine rules + board rendering +
modes + dice + snakes/ladders + win/overshoot + AI + restart/exit/nav, in real
Chromium via Playwright. Mirrors scripts/verify_ludo.py."""
import json, sys, time
from playwright.sync_api import sync_playwright

BASE = "http://127.0.0.1:8911"
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

    # ---------- ENGINE RULES (pure, deterministic) ----------
    page.evaluate("location.hash = '#funtime/sn'")
    page.wait_for_selector("#snBoard")
    page.wait_for_timeout(400)
    page.evaluate("window.SND.fast()")

    r = page.evaluate("""(() => {
      const E = window.SnakesEngine, out = {};
      // zigzag geometry
      out.sq1 = E.CELL[1].join(',');
      out.sq10 = E.CELL[10].join(',');
      out.sq11 = E.CELL[11].join(',');
      out.sq20 = E.CELL[20].join(',');
      out.sq100 = E.CELL[100].join(',');
      const seen = new Set();
      for (let n=1;n<=100;n++) seen.add(E.CELL[n].join(','));
      out.unique100 = seen.size === 100;
      // dice range over 20000 rolls
      let bad = 0;
      for (let i=0;i<20000;i++){ const v=E.rollDice(); if(v<1||v>6||v!==Math.floor(v)) bad++; }
      out.diceRange = bad === 0;
      // fresh game: everyone off board, red opens
      const g = E.createGame(['human','ai']);
      out.startOffBoard = g.players.every(P => P.pos === 0);
      out.redOpens = g.turn === 0;
      // illegal rolls rejected
      out.rejectZero = E.previewMove(g,0,0) === null;
      out.rejectSeven = E.previewMove(g,0,7) === null;
      out.rejectWrongPlayer = E.previewMove(g,1,3) === null;
      // entry on first roll
      const g2 = E.createGame(['human','ai']);
      const m2 = E.applyRoll(g2,5);
      out.enterOnFirstRoll = m2.from===0 && m2.to===5 && g2.players[0].pos===5;
      // every ladder
      out.ladders = E.LADDERS.map(([b,t]) => {
        const gg = E.createGame(['human','ai']); gg.players[0].pos=b-1;
        return E.applyRoll(gg,1).final === t;
      }).every(Boolean);
      // every snake
      out.snakes = E.SNAKES.map(([h,t]) => {
        const gg = E.createGame(['human','ai']); gg.players[0].pos=h-1;
        return E.applyRoll(gg,1).final === t;
      }).every(Boolean);
      // overshoot
      const g3 = E.createGame(['human','ai']); g3.players[0].pos=98;
      const m3 = E.applyRoll(g3,3);
      out.overshootStays = m3.overshoot===true && g3.players[0].pos===98 && g3.turn===1;
      // exact win
      const g4 = E.createGame(['human','ai']); g4.players[0].pos=97;
      const m4 = E.applyRoll(g4,3);
      out.exactWin = m4.won===true && g4.players[0].pos===100 && g4.winner===0;
      // near-win snake at 98
      const g5 = E.createGame(['human','ai']); g5.players[0].pos=95;
      const m5 = E.applyRoll(g5,3);
      out.snakeAt98 = g5.players[0].pos===78 && m5.kind==='snake';
      // turn cycling 4 players
      const g6 = E.createGame(['human','human','human','human']);
      for (let i=0;i<4;i++) E.applyRoll(g6,1);
      out.turnCycle4 = g6.turn===0 && g6.rolls===4;
      // ai gating
      const g7 = E.createGame(['human','ai']);
      out.aiGate = E.aiShouldRoll(g7)===false && (g7.turn=1, E.aiShouldRoll(g7)===true);
      return out;
    })()""")
    for k, v in r.items():
        if k in ("sq1","sq10","sq11","sq20","sq100"):
            check("engine/" + k, v in ("0,0","0,9","1,9","1,0","9,0"), str(v))
        else:
            check("engine/" + k, v)

    # ---------- BOARD RENDERS ----------
    box = page.evaluate("""(() => {
      const c = document.getElementById('snBoard');
      const r = c.getBoundingClientRect();
      return {w:r.width, h:r.height, ratio:r.width/r.height};
    })()""")
    check("board/square", abs(box["ratio"] - 1.0) < 0.05, f"ratio={box['ratio']:.3f}")
    check("board/visible", box["w"] > 300, f"w={box['w']:.0f}px")
    px = page.evaluate("""(() => {
      const c = document.getElementById('snBoard');
      const ctx = c.getContext('2d');
      const d = ctx.getImageData(0,0,c.width,c.height).data;
      let n=0; for(let i=3;i<d.length;i+=4*97){ if(d[i]>0) n++; }
      return n;
    })()""")
    check("board/painted", px > 200, f"{px} sampled non-transparent px")
    # all 100 numbers drawn? sample canvas text by checking engine CELL count
    check("board/100squares", page.evaluate("window.SnakesEngine.CELL.length") == 101)

    # ---------- MODE SELECTION ----------
    modes = page.evaluate("Object.keys(window.SN_MODES)")
    check("modes/allPresent", set(modes) == {"pvc","2p","3p","4p","2vc"}, ",".join(sorted(modes)))
    for m, want in [("2p",2),("3p",3),("4p",4),("2vc",3),("pvc",2)]:
        page.evaluate(f"window.SNG.mode = '{m}'")
        page.click("#snStart")
        page.wait_for_timeout(150)
        n = page.evaluate("window.SNG.game.players.length")
        kinds = page.evaluate("window.SNG.game.players.map(p=>p.kind)")
        ok = n == want
        if m == "2vc": ok = ok and kinds == ["human","human","ai"]
        if m == "pvc": ok = ok and kinds == ["human","ai"]
        check(f"mode/{m}", ok, f"{n} players {kinds}")

    # ---------- HUMAN TURN: roll + move ----------
    page.evaluate("window.SNG.mode='2p'")
    page.click("#snStart"); page.wait_for_timeout(150)
    page.evaluate("window.SND.force(4)")
    page.wait_for_timeout(900)
    st = page.evaluate("({turn: window.SNG.game.turn, pos: window.SNG.game.players[0].pos, phase: window.SNG.phase})")
    # 0+4 lands on square 4 = ladder bottom -> climbs to 14
    check("human/moved4climbsTo14", st["pos"] == 14 and st["turn"] == 1, json.dumps(st))

    # plain move with no jump: force from square 15 (no snake/ladder at 17..21? 21 is a ladder bottom)
    page.evaluate("window.SNG.game.players[1].pos = 15")
    page.evaluate("window.SNG.game.turn = 1")
    page.evaluate("window.SND.force(2)")   # 15+2=17, plain square
    page.wait_for_timeout(900)
    st = page.evaluate("({pos: window.SNG.game.players[1].pos, turn: window.SNG.game.turn})")
    check("human/plainMoveTo17", st["pos"] == 17 and st["turn"] == 0, json.dumps(st))

    # ---------- LADDER CLIMB (player 2 from 5 -> 9 -> 31) ----------
    page.evaluate("window.SNG.game.players[1].pos = 5")
    page.evaluate("window.SNG.game.turn = 1")     # hand the dice to Green
    page.evaluate("window.SND.force(4)")   # 5+4=9 -> ladder to 31
    page.wait_for_timeout(2200)
    st = page.evaluate("({pos: window.SNG.game.players[1].pos, turn: window.SNG.game.turn})")
    check("ladder/9to31", st["pos"] == 31 and st["turn"] == 0, json.dumps(st))

    # ---------- SNAKE SLIDE (player 1 from 43 -> 47 head -> 26) ----------
    page.evaluate("window.SNG.game.players[0].pos = 43")
    page.evaluate("window.SND.force(4)")   # 43+4=47 -> snake to 26
    page.wait_for_timeout(2200)
    st = page.evaluate("({pos: window.SNG.game.players[0].pos})")
    check("snake/47to26", st["pos"] == 26, json.dumps(st))

    # ---------- OVERSHOOT ----------
    page.evaluate("window.SNG.game.players[1].pos = 98")
    page.evaluate("window.SND.force(4)")
    page.wait_for_timeout(1200)
    st = page.evaluate("({pos: window.SNG.game.players[1].pos, turn: window.SNG.game.turn})")
    check("overshoot/staysAt98", st["pos"] == 98, json.dumps(st))

    # ---------- EXACT WIN ----------
    page.evaluate("window.SNG.game.players[0].pos = 96")
    page.evaluate("window.SND.force(4)")   # 96+4=100 exactly
    page.wait_for_timeout(1600)
    st = page.evaluate("({winner: window.SNG.game.winner, pos: window.SNG.game.players[0].pos, overlay: !document.getElementById('snOverlay').classList.contains('hidden')})")
    check("win/exact100", st["winner"] == 0 and st["pos"] == 100 and st["overlay"], json.dumps(st))
    title = page.evaluate("document.getElementById('snOvTitle').textContent")
    check("win/celebration", len(title) > 0, title)

    # ---------- RESTART KILLS STALE CALLBACKS ----------
    pre_epoch = page.evaluate("window.SNG.epoch")
    page.click("#snRestartBtn")
    page.wait_for_timeout(300)
    post = page.evaluate("({epoch: window.SNG.epoch, rolls: window.SNG.game.rolls, turn: window.SNG.game.turn, winner: window.SNG.game.winner, pos: window.SNG.game.players[0].pos, overlay: document.getElementById('snOverlay').classList.contains('hidden')})")
    check("restart/freshState", post["rolls"] == 0 and post["turn"] == 0 and post["winner"] is None
          and post["pos"] == 0 and post["overlay"], json.dumps(post))
    # restart must work from ANY state, including mid-animation
    page.evaluate("window.SND.force(6)")
    page.wait_for_timeout(80)                    # mid-walk
    page.click("#snRestartBtn")
    page.wait_for_timeout(300)
    post2 = page.evaluate("({rolls: window.SNG.game.rolls, phase: window.SNG.phase, anim: window.SNG.anim, viz: window.SNG.viz})")
    check("restart/midAnimation", post2["rolls"] == 0 and post2["anim"] is None and post2["viz"] is None, json.dumps(post2))

    # ---------- COMPUTER MODE: AI RUNS AUTONOMOUSLY ----------
    page.evaluate("window.SNG.mode='pvc'")
    page.click("#snStart"); page.wait_for_timeout(200)
    deadline = time.time() + 25
    ai_done = False
    while time.time() < deadline:
        s = page.evaluate("({turn: window.SNG.game.turn, rolls: window.SNG.game.rolls, winner: window.SNG.game.winner, phase: window.SNG.phase})")
        if s["winner"] is not None or (s["turn"] == 1 and s["rolls"] > 0):
            ai_done = True
            break
        # autoplay human turns so the AI gets its chance
        if s["turn"] == 0 and s["phase"] == "idle":
            page.evaluate("window.SND.force(Math.floor(Math.random()*6)+1)")
        page.wait_for_timeout(120)
    check("ai/autonomousRoll", ai_done, json.dumps(s))

    # full AI-vs-AI-ish game to completion via forced rolls (2-player mode)
    page.evaluate("window.SNG.mode='2p'")
    page.click("#snStart"); page.wait_for_timeout(150)
    deadline = time.time() + 60
    won = False
    while time.time() < deadline:
        s = page.evaluate("({winner: window.SNG.game.winner, phase: window.SNG.phase, turn: window.SNG.game.turn})")
        if s["winner"] is not None:
            won = True
            break
        if s["phase"] == "idle":
            page.evaluate("window.SND.force(Math.floor(Math.random()*6)+1)")
        page.wait_for_timeout(60)
    check("game/completes", won, f"winner={s.get('winner')}")
    if won:
        wpos = page.evaluate("window.SNG.game.players[window.SNG.game.winner].pos")
        check("game/winnerOn100", wpos == 100, str(wpos))

    # ---------- EXIT / NAV ----------
    page.evaluate("location.hash = '#funtime'")
    page.wait_for_timeout(500)
    cards = page.evaluate("document.querySelectorAll('.ft-card').length")
    check("nav/backToFuntime", cards >= 10, f"{cards} cards visible")
    check("nav/snakesCardPresent", page.evaluate("!!document.getElementById('ftCardSn')"))
    for cid in ["ftCardT3","ftCardSi","ftCardHt","ftCardCr","ftCardCh","ftCardTm","ftCardSo","ftCardFn","ftCardLu"]:
        check(f"nav/{cid}", page.evaluate(f"!!document.getElementById('{cid}')"))

    # ---------- CARD CLICK OPENS THE GAME ----------
    page.click("#ftCardSn")
    page.wait_for_timeout(600)
    check("nav/cardLaunchesGame", page.evaluate("!!document.getElementById('snBoard')"))

    # ---------- ESC RETURNS TO SELECTOR (session behaviour) ----------
    page.keyboard.press("Escape")
    page.wait_for_timeout(500)
    check("nav/escExits", page.evaluate("!!document.getElementById('ftCardSn')"),
          "selector visible after Esc")

    # ---------- FULLSCREEN ----------
    page.set_viewport_size({"width": 1360, "height": 950})
    page.evaluate("location.hash = '#funtime/sn'")
    page.wait_for_selector("#snBoard"); page.wait_for_timeout(400)
    inline_w = page.evaluate("Math.round(document.getElementById('snBoard').getBoundingClientRect().width)")
    page.click("#snFsBtn")
    page.wait_for_timeout(900)
    fs = page.evaluate("""(() => ({
      native: !!document.fullscreenElement,
      w: Math.round(document.getElementById('snBoard').getBoundingClientRect().width),
      headerHidden: getComputedStyle(document.querySelector('#snPanel .ft-panel-h')).display === 'none'}))()""")
    check("fullscreen/nativeEntered", fs["native"])
    check("fullscreen/boardGrows", fs["w"] > inline_w, f"inline={inline_w} fs={fs['w']}")
    check("fullscreen/headerHidden", fs["headerHidden"])
    page.screenshot(path="/tmp/sn-shots/7-fullscreen.png")
    # Esc leaves fullscreen AND the game (shared session behaviour)
    page.keyboard.press("Escape")
    page.wait_for_timeout(700)
    esc = page.evaluate("""(() => ({
      native: !!document.fullscreenElement,
      selector: !!document.getElementById('ftCardSn'),
      bodyClean: document.body.className === ''}))()""")
    check("fullscreen/escExitsGame", esc["selector"] and not esc["native"], json.dumps(esc))
    check("fullscreen/bodyClassRestored", esc["bodyClean"], esc["bodyClean"])

    # card click -> automatic native fullscreen
    page.click("#ftCardSn")
    page.wait_for_timeout(1000)
    cl = page.evaluate("""(() => ({
      native: !!document.fullscreenElement,
      playing: document.body.classList.contains('ft-playing'),
      board: !!document.getElementById('snBoard')}))()""")
    check("fullscreen/cardAutoEnters", cl["native"] and cl["playing"] and cl["board"], json.dumps(cl))
    page.keyboard.press("Escape")
    page.wait_for_timeout(700)

    # ---------- RESPONSIVE ----------
    for w, h in [(390, 844), (768, 1024), (1920, 1080)]:
        page.set_viewport_size({"width": w, "height": h})
        page.evaluate("location.hash = '#funtime/sn'")
        page.wait_for_selector("#snBoard")
        page.wait_for_timeout(350)
        b = page.evaluate("""(() => {
          const c = document.getElementById('snBoard');
          const r = c.getBoundingClientRect();
          return {w:r.width, vis: r.width>0 && r.height>0,
                  inVp: r.top >= -1 && r.left >= -1 && r.right <= innerWidth+1 && r.bottom <= innerHeight+1};
        })()""")
        check(f"responsive/{w}x{h}", b["vis"] and b["inVp"], json.dumps(b))

    # ---------- VISUAL: snakes & ladders painted at their real positions ----------
    vis = page.evaluate("""(() => {
      const E = window.SnakesEngine;
      const c = document.getElementById('snBoard');
      const ctx = c.getContext('2d');
      const C = c.width/10;
      const ctr = n => { const rc=E.CELL[n]; return {x:(rc[1]+.5)*C, y:((9-rc[0])+.5)*C}; };
      const isAmber = d => d[0]>190&&d[1]>150&&d[2]<120;
      const isRed = d => d[0]>180&&d[1]<130&&d[2]<140;
      function ladderCheck(b, t){
        const A=ctr(b), B=ctr(t);
        const dx=B.x-A.x, dy=B.y-A.y, len=Math.hypot(dx,dy);
        const ux=dx/len, uy=dy/len, px=-uy, py=ux;
        let hits=0, tries=0;
        for (let d=len*0.15; d<=len*0.85; d+=len*0.1){
          for (const s of [-1,1]){
            const x=A.x+ux*d+px*C*0.19*s, y=A.y+uy*d+py*C*0.19*s;
            const d1=ctx.getImageData(Math.round(x),Math.round(y),1,1).data;
            tries++; if (isAmber(d1)) hits++;
          }
        }
        return hits/(tries||1);
      }
      function snakeHead(h){
        const H=ctr(h); let hits=0, tries=0;
        for(let dx=-8;dx<=8;dx+=2)for(let dy=-8;dy<=8;dy+=2){
          const d=ctx.getImageData(Math.round(H.x+dx),Math.round(H.y+dy),1,1).data;
          tries++; if(isRed(d)) hits++;
        }
        return hits/(tries||1);
      }
      // every ladder rail + every snake head must be painted
      const ladders = E.LADDERS.every(([b,t]) => ladderCheck(b,t) > 0.2);
      const heads = E.SNAKES.every(([h]) => snakeHead(h) > 0.25);
      // all 100 numbers legible in their cells
      let withText = 0;
      for (let n=1;n<=100;n++){
        const rc=E.CELL[n];
        const x=rc[1]*C+C*0.05, y=(9-rc[0])*C+C*0.05;
        const d=ctx.getImageData(x,y,C*0.5,C*0.35).data;
        let lit=0;
        for (let i=0;i<d.length;i+=4){ if (d[i]>110&&d[i+1]>110&&d[i+2]>110) lit++; }
        if (lit>4) withText++;
      }
      return {ladders, heads, withText};
    })()""")
    check("visual/allLaddersPainted", vis["ladders"])
    check("visual/allSnakeHeadsPainted", vis["heads"])
    check("visual/allNumbersLegible", vis["withText"] == 100, f"{vis['withText']}/100")

    # ---------- CONSOLE ERRORS ----------
    check("browser/noConsoleErrors", len(errors) == 0, "; ".join(errors[:3]))

    browser.close()

fails = [r for r in RESULTS if not r[1]]
print(f"\n==== {len(RESULTS)-len(fails)}/{len(RESULTS)} passed ====")
sys.exit(1 if fails else 0)
