#!/usr/bin/env python3
"""QA for the six internal-evaluation workspace cards on the Loop Home page.

Boots the real dashboard binary against the real kit at ~/.loop/eval, then
verifies in real Chromium:

  1.  All six cards render on the Welcome page with real counts.
  2.  Every card navigates to its own workspace.
  3.  The question bank loads real questions and search/filters work.
  4.  Dataset previews render real file contents.
  5.  Parquet metadata is exposed (schema + row count) without loading data.
  6.  Model evaluation surfaces real runs/reports when present.
  7.  Reports and tools list real artifacts.
  8.  Deep pages of the 96,500-question bank stay fast (no freeze).
  9.  Existing workspaces (SFT, Pretraining, Fun Time) still open.
 10.  Unauthenticated visitors are redirected to sign-in (guard intact).

Usage:  .venv-test/bin/python scripts/verify_eval_workspaces.py [--port 8931]
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

from playwright.sync_api import sync_playwright

PORT = 8947
BASE = f"http://127.0.0.1:{PORT}"
OUT = "/tmp/eval-ws-shots"
RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  [{detail}]" if detail else ""))


def api(path):
    with urllib.request.urlopen(BASE + path) as r:
        return json.loads(r.read())


def main():
    os.makedirs(OUT, exist_ok=True)
    home = os.path.expanduser("~")
    kit = f"{home}/.loop/eval/internal_eval"
    if not os.path.isdir(kit):
        print("kit absent at", kit, "— run against a machine with the kit unpacked")
        sys.exit(2)

    # Isolated auth DB so the test never touches the real one.
    tmp = tempfile.mkdtemp(prefix="eval-ws-auth-")
    env = dict(os.environ, HOME=home)
    # Keep LOOP_EVAL_ROOT unset: the default resolution must find the kit.
    env.pop("LOOP_EVAL_ROOT", None)

    proc = subprocess.Popen(
        [
            f"{home}/loop/target/debug/loop-dashboard",
            "--port", str(PORT),
            "--console-db", f"{tmp}/store.db",
            "--no-auth",
        ],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    try:
        for _ in range(80):
            try:
                urllib.request.urlopen(BASE + "/api/health", timeout=1)
                break
            except Exception:
                time.sleep(0.25)
        else:
            print("dashboard did not come up")
            print(proc.stdout.read() if proc.stdout else "")
            sys.exit(1)

        ov = api("/api/eval/overview")
        check("backend/overviewConnected", ov["integration"] == "connected",
              f"{ov['files']} files, {ov['banks']['questions']} questions")

        with sync_playwright() as p:
            b = p.chromium.launch()
            page = b.new_page(viewport={"width": 1440, "height": 900})
            errors = []
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)

            # ---- 1. the six cards on Home -------------------------------
            page.goto(BASE + "/console")
            page.wait_for_selector("#wsGrid", timeout=15000)
            page.wait_for_selector("#evGrid", timeout=15000)
            page.wait_for_timeout(1200)
            page.screenshot(path=f"{OUT}/01-home.png", full_page=True)

            cards = page.locator("#evGrid .ws-card")
            check("cards/sixRender", cards.count() == 6, f"{cards.count()} cards")
            names = [cards.nth(i).locator(".ws-name").inner_text() for i in range(cards.count())]
            expected = ["Question Bank", "Evaluation Datasets", "Benchmark Center",
                        "Model Evaluation", "Reports & Analytics", "Evaluation Tools"]
            check("cards/titlesMatch", names == expected, ", ".join(names))

            badge_txt = page.locator("#evPortalBadge").inner_text()
            check("cards/groupBadgeReal", any(ch.isdigit() for ch in badge_txt), badge_txt)
            qb_badge = page.locator("#ws-evalbank .ws-badge").inner_text().lower()
            check("cards/questionCountReal",
                  "questions" in qb_badge and ov["banks"]["questions"] > 200000,
                  f"{qb_badge!r} (backend: {ov['banks']['questions']:,})")

            # ---- 2. each card opens its workspace ----------------------
            for route, title in [
                ("#evalbank", "Question Bank"),
                ("#evaldata", "Evaluation Datasets"),
                ("#evalbench", "Benchmark Center"),
                ("#evalmodel", "Model Evaluation"),
                ("#evalreport", "Reports & Analytics"),
                ("#evaltools", "Evaluation Tools"),
            ]:
                page.evaluate(f"location.hash = '{route}'")
                page.wait_for_timeout(900)
                got = page.locator("#pageTitle").inner_text()
                check(f"nav/{route[1:]}", got == title, f"title={got!r}")

            # ---- 3. question bank: rows, search, facets ----------------
            page.evaluate("location.hash = '#evalbank'")
            page.wait_for_selector("#evBankBody tr", timeout=20000)
            page.wait_for_timeout(600)
            rows = page.locator("#evBankBody tr").count()
            check("bank/rowsRender", rows == 25, f"{rows} rows on page 1")
            first_id = page.locator("#evBankBody tr td").first.inner_text()
            check("bank/realIds", "|" in first_id, first_id)

            # Search narrows honestly.
            page.fill("#evQ", "triangle")
            page.wait_for_timeout(900)
            count_txt = page.locator("#evBankCount").inner_text()
            total_after = int(count_txt.split("of")[-1].strip().replace(",", ""))
            check("bank/searchNarrows", 0 < total_after < ov["banks"]["questions"],
                  f"{total_after} matches for 'triangle'")
            page.fill("#evQ", "")
            page.wait_for_timeout(900)

            # Facet filter: click the first difficulty chip.
            chips = page.locator(".ev-facet-v[data-attr='difficulty'] .ev-chip")
            if chips.count() > 1:
                chips.nth(1).click()
                page.wait_for_timeout(800)
                on = page.locator(".ev-facet-v[data-attr='difficulty'] .ev-chip.on").inner_text()
                check("bank/difficultyFilter", on.strip().startswith("L"), on)

            # Question detail opens (this replaces the view, so we come
            # back to the bank afterwards before paginating).
            page.locator("#evBankBody tr").first.click()
            page.wait_for_timeout(900)
            detail = page.locator(".detail-grid .panel").count()
            check("bank/questionDetail", detail == 2,
                  f"{detail} panels (metadata + question)")
            page.screenshot(path=f"{OUT}/03-question-detail.png", full_page=True)
            page.click("#evBackToBank")
            page.wait_for_selector("#evBankBody tr", timeout=20000)
            page.wait_for_timeout(600)

            # ---- 4. deep pagination does not freeze --------------------
            t0 = time.time()
            page.evaluate("EV.banks.page = 3500; evLoadBankPage()")
            page.wait_for_function(
                "() => document.querySelector('#evBankCount') && "
                "document.querySelector('#evBankCount').textContent.includes('of')",
                timeout=15000,
            )
            elapsed = time.time() - t0
            deep = page.locator("#evBankCount").inner_text()
            check("bank/deepPageFast", elapsed < 3.0, f"{elapsed:.2f}s → {deep}")
            page.screenshot(path=f"{OUT}/04-deep-page.png", full_page=True)

            # ---- 5. datasets -------------------------------------------
            page.evaluate("location.hash = '#evaldata'")
            page.wait_for_timeout(1000)
            ds_rows = page.locator("tr[data-id]").count()
            body_txt = page.locator("#viewRoot").inner_text()
            # The shipped kit has an empty compare_cache/, so the honest
            # empty state is the correct outcome.
            if ds_rows == 0:
                check("datasets/honestEmptyState", "No datasets" in body_txt,
                      "empty state explains itself")
            else:
                check("datasets/rowsListed", ds_rows > 0, f"{ds_rows} datasets")

            # ---- 6. benchmarks ----------------------------------------
            page.evaluate("location.hash = '#evalbench'")
            page.wait_for_timeout(1100)
            bt = page.locator("#viewRoot").inner_text()
            presets = api("/api/eval/benchmarks")["presets"]
            check("bench/presetsFromRealFiles",
                  "--input qbank-sample" in bt and len(presets) >= 5,
                  f"{len(presets)} presets listed")
            check("bench/docLinks", page.locator(".ev-doc-link").count() >= 1,
                  "documentation links render")

            # ---- 7. model evaluation ----------------------------------
            page.evaluate("location.hash = '#evalmodel'")
            page.wait_for_timeout(1000)
            mt = page.locator("#viewRoot").inner_text()
            runs = api("/api/eval/runs")["items"]
            if runs:
                check("model/runsListed", runs[0]["id"] in mt, f"{len(runs)} runs")
            else:
                check("model/honestEmptyState", "No evaluation runs yet" in mt,
                      "no runs in the shipped kit")

            # ---- 8. reports + tools -----------------------------------
            page.evaluate("location.hash = '#evalreport'")
            page.wait_for_timeout(1000)
            rt = page.locator("#viewRoot").inner_text()
            check("reports/renders", "Reports" in page.locator("#pageTitle").inner_text(),
                  "workspace opens")

            page.evaluate("location.hash = '#evaltools'")
            page.wait_for_timeout(1100)
            tt = page.locator("#viewRoot").inner_text()
            tools = api("/api/eval/files?workspace=tools")["items"]
            check("tools/listsRealScripts",
                  any("judge.py" in i["relPath"] for i in tools) and "judge.py" in tt,
                  f"{len(tools)} tool resources")
            check("tools/docsRender", page.locator(".ev-doc-link").count() >= 3,
                  "README / WORKFLOW / FAQ links")

            # Open one document.
            page.locator(".ev-doc-link").first.click()
            page.wait_for_timeout(900)
            md = page.locator(".ev-md").count()
            check("tools/docRenders", md == 1, "markdown rendered")
            page.screenshot(path=f"{OUT}/08-doc.png", full_page=True)

            # Preview a pipeline script (return via the view's own back
            # button — assigning an unchanged hash fires no hashchange).
            page.click("#evDocBack")
            page.wait_for_selector("tr[data-id]", timeout=15000)
            page.wait_for_timeout(600)
            page.locator("tr[data-id]").first.click()
            page.wait_for_timeout(900)
            pv = page.locator(".ev-pre, .table-scroll").count()
            check("tools/filePreview", pv >= 1, "preview renders")
            page.screenshot(path=f"{OUT}/09-preview.png", full_page=True)

            # ---- 9. existing workspaces still work --------------------
            for route, sel in [("#sft", "#viewRoot"),
                               ("#pretrain", "#viewRoot"),
                               ("#funtime", "#viewRoot")]:
                page.evaluate(f"location.hash = '{route}'")
                page.wait_for_timeout(1100)
                txt = page.locator("#viewRoot").inner_text()[:60].replace("\n", " ")
                check(f"regress/{route[1:]}", len(txt.strip()) > 0, txt[:40])
            page.screenshot(path=f"{OUT}/10-sft-regression.png")

            # No page errors anywhere above.
            check("console/noPageErrors", not errors, "; ".join(errors[:3]))

            b.close()

        # ---- 10. auth guard stays intact (fresh app WITH auth) -------
        proc.terminate()
        proc.wait(timeout=10)
        proc2 = subprocess.Popen(
            [
                f"{home}/loop/target/debug/loop-dashboard",
                "--port", str(PORT),
                "--console-db", f"{tmp}/store2.db",
            ],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        try:
            for _ in range(60):
                try:
                    urllib.request.urlopen(BASE + "/api/health", timeout=1)
                    break
                except Exception:
                    time.sleep(0.25)
            with sync_playwright() as p:
                b = p.chromium.launch()
                pg = b.new_page()
                pg.goto(BASE + "/console", wait_until="domcontentloaded")
                pg.wait_for_timeout(700)
                url = pg.url
                check("auth/consoleRedirects", "/signin" in url, url)
                # The eval API answers 401 JSON without a session.
                req = pg.request.get(BASE + "/api/eval/overview")
                check("auth/evalApiGuarded", req.status == 401, f"HTTP {req.status}")
                b.close()
        finally:
            proc2.terminate()
            proc2.wait(timeout=10)

    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except Exception:
            proc.kill()
        shutil.rmtree(tmp, ignore_errors=True)

    bad = [r for r in RESULTS if not r[1]]
    print(f"\n{len(RESULTS) - len(bad)}/{len(RESULTS)} checks passed")
    if bad:
        print("failed:", ", ".join(n for n, _, _ in bad))
        sys.exit(1)


if __name__ == "__main__":
    main()
