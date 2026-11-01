'use strict';
/* ============================================================
   INTERNAL EVALUATION WORKSPACES (routes: evalbank, evaldata,
   evalbench, evalmodel, evalreport, evaltools)
   Real source: /api/eval/* over the internal_eval kit at ~/.loop/eval.
   Read-only. Every number comes from a real file on disk; nothing is
   simulated or invented. The kit's own submit.sh / sbatch runners stay
   authoritative — this surface only reads what they produced.
   ============================================================ */

/* Shared state for the six workspaces. Kept separate from the main S so a
 * failing eval fetch can never disturb the console's own state. */
const EV = {
  overview: null,
  banks: { items: [], sel: null, page: 1, perPage: 25, q: '',
           suite: 'all', group: 'all', difficulty: 'all', language: 'all' },
  files: { items: [] },
  runs: { items: [] },
};

/* ---------- formatting helpers (local to the eval surfaces) ---------- */
const evFmtInt = n => (n ?? 0).toLocaleString('en-US');
const evFmtBytes = (n, dp = 1) => {
  if (n == null || !isFinite(n)) return '—';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0, v = n;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return (i === 0 ? Math.round(v) : v.toFixed(dp)) + ' ' + u[i];
};
const evFmtPct = (v, dp = 1) =>
  (v == null || !isFinite(v)) ? '—' : Number(v).toFixed(dp) + '%';
const evAgo = ms => {
  if (!ms) return '—';
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 60) return Math.round(s) + 's ago';
  if (s < 3600) return Math.round(s / 60) + 'm ago';
  if (s < 86400) return Math.round(s / 3600) + 'h ago';
  return Math.round(s / 86400) + 'd ago';
};

/* A small badge, matching the portal's ws-badge vocabulary. */
function evBadge(text, kind) {
  return `<span class="ws-badge ${kind}">${esc(text)}</span>`;
}

/* Facet chips: key + count, clickable when a handler is given. */
function evChips(buckets, onPick, active) {
  if (!buckets || !buckets.length) return '<span class="faint mono" style="font-size:11px">none</span>';
  return buckets.map(b =>
    `<button type="button" class="ev-chip${active === b.key ? ' on' : ''}"
      data-k="${esc(b.key)}" title="${evFmtInt(b.count)} questions">
      ${esc(b.key || '—')}<span>${evFmtInt(b.count)}</span></button>`).join('');
}

/* Honest empty state shared by all six workspaces. */
function evEmpty(title, hint) {
  return `<div class="empty" style="padding:34px 18px">
    <div class="empty-t">${esc(title)}</div>
    <div class="empty-s">${esc(hint || '')}</div></div>`;
}

/* The "kit not found" panel — mirrors notConfiguredPanel's honesty. */
function evNotConfigured(reason) {
  return `<div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Internal evaluation — not configured on this machine</div>
      <div class="panel-sub">integration state: not configured</div>
    </div></div>
    <div style="padding:22px 24px">
      <div style="font-family:var(--mono);font-size:12px;color:var(--ink-dim);line-height:1.8;max-width:780px">
        ${esc(reason || 'The internal_eval kit was not found.')}
      </div>
      <div style="margin-top:16px;font-family:var(--mono);font-size:11px;color:var(--ink-faint);line-height:1.9">
        These workspaces read the evaluation kit at
        <code>~/.loop/eval/internal_eval</code> (question banks, pipeline
        scripts, run outputs). Nothing is executed and no status is invented:
        once the kit exists there, the workspaces populate automatically.
        Point elsewhere with <code>LOOP_EVAL_ROOT</code>.
      </div>
    </div></div>`;
}

/* Load the shared overview once; every workspace may use it. */
async function evOverview(force) {
  if (EV.overview && !force) return EV.overview;
  EV.overview = await api('/api/eval/overview');
  return EV.overview;
}

/* ============================================================
   1 · QUESTION BANK  (route: evalbank)
   Browse and search the kit's question banks: facets, filters,
   pagination, full question detail with answer keys.
   ============================================================ */
async function renderEvalBank(sub) {
  $('#pageTitle').textContent = 'Question Bank';
  $('#pageSub').textContent = 'browse and search the internal evaluation question banks';

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="sk-line"></div><div class="sk-line short"></div></div></div>`;

  let list;
  try {
    list = await api('/api/eval/banks');
  } catch (e) { return errorView(e); }

  if (!list.items || !list.items.length) {
    root.innerHTML = `<div class="content"><div class="content-inner">
      ${list.integration === 'not_configured'
        ? evNotConfigured('No question banks were found in the evaluation kit.')
        : `<div class="panel"><div class="panel-h"><div>
             <div class="panel-t">Question Bank</div>
             <div class="panel-sub">no bank files discovered</div></div></div>
           <div style="padding:20px">${evEmpty('No question banks',
             'Place .jsonl question files under internal_eval/banks/ to see them here.')}</div></div>`}
    </div></div>`;
    return;
  }

  // Deep link: #evalbank/<file>
  const wanted = sub && list.items.find(b => b.name === sub);
  if (wanted) EV.banks.sel = wanted.name;
  if (!EV.banks.sel || !list.items.some(b => b.name === EV.banks.sel)) {
    EV.banks.sel = list.items[0].name;
  }

  const bank = list.items.find(b => b.name === EV.banks.sel);

  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="kpis">
      <div class="kpi"><div class="kpi-k">questions indexed</div>
        <div class="kpi-v">${evFmtInt(list.items.reduce((a, b) => a + b.rows, 0))}</div>
        <div class="kpi-s">across ${list.items.length} bank file${list.items.length === 1 ? '' : 's'}</div></div>
      <div class="kpi"><div class="kpi-k">this bank</div>
        <div class="kpi-v">${evFmtInt(bank.rows)}</div>
        <div class="kpi-s">${esc(bank.name)} · ${evFmtBytes(bank.bytes)}</div></div>
      <div class="kpi"><div class="kpi-k">with answer key</div>
        <div class="kpi-v">${evFmtInt(bank.withReference)}</div>
        <div class="kpi-s">${evFmtInt(bank.withOptions)} carry options</div></div>
      <div class="kpi"><div class="kpi-k">subjects · topics</div>
        <div class="kpi-v">${evFmtInt(bank.subjects)} <span class="dim" style="font-size:14px">/</span> ${evFmtInt(bank.topics)}</div>
        <div class="kpi-s">distinct classification values</div></div>
    </div>

    <div class="filters">
      <span class="filter-label">bank</span>
      <select class="ev-select" id="evBankSel">
        ${list.items.map(b => `<option value="${esc(b.name)}"${b.name === EV.banks.sel ? ' selected' : ''}>
          ${esc(b.name)} (${evFmtInt(bank_rows(b))})</option>`).join('')}
      </select>
      <span class="filter-label">search</span>
      <div class="search-box"><input id="evQ" placeholder="question text, id, topic…"
        value="${esc(EV.banks.q)}"></div>
      <button class="adv-toggle" id="evReset">reset filters</button>
    </div>

    <div class="ev-facets" id="evFacets"></div>

    <div class="panel">
      <div class="table-scroll"><table>
        <thead><tr>
          <th>id</th><th>group</th><th>subject</th><th>topic</th>
          <th>difficulty</th><th>type</th><th>question</th>
        </tr></thead>
        <tbody id="evBankBody"></tbody>
      </table></div>
      <div class="tbl-foot">
        <span class="tbl-count" id="evBankCount"></span>
        <div class="pager" id="evBankPager"></div>
      </div>
    </div>
    <div class="pa-note-line">Served from <code>${esc(bank.relPath)}</code> through
      <code>/api/eval/banks/${esc(bank.name)}/page</code> · pages read only the rows
      they show, so browsing a 96,500-question bank costs one bounded request.</div>
  </div></div>`;

  function bank_rows(b) { return b.rows; }

  $('#evBankSel').onchange = e => {
    EV.banks.sel = e.target.value;
    EV.banks.page = 1;
    EV.banks.q = ''; EV.banks.suite = 'all'; EV.banks.group = 'all';
    EV.banks.difficulty = 'all'; EV.banks.language = 'all';
    location.hash = '#evalbank/' + encodeURIComponent(EV.banks.sel);
    renderEvalBank(EV.banks.sel);
  };
  $('#evQ').oninput = debounce(() => {
    EV.banks.q = $('#evQ').value; EV.banks.page = 1; evLoadBankPage();
  }, 250);
  $('#evReset').onclick = () => {
    EV.banks.q = ''; EV.banks.suite = 'all'; EV.banks.group = 'all';
    EV.banks.difficulty = 'all'; EV.banks.language = 'all'; EV.banks.page = 1;
    $('#evQ').value = '';
    evRenderFacets(bank);
    evLoadBankPage();
  };

  evRenderFacets(bank);
  await evLoadBankPage();
}

/* Render the facet filter row for the selected bank. */
function evRenderFacets(bank) {
  const wrap = $('#evFacets');
  if (!wrap) return;
  const mk = (label, values, current, attr) => `
    <div class="ev-facet">
      <div class="ev-facet-k">${label}</div>
      <div class="ev-facet-v" data-attr="${attr}">
        <button type="button" class="ev-chip${current === 'all' ? ' on' : ''}" data-k="all">all</button>
        ${values.map(v => `<button type="button" class="ev-chip${current === v.key ? ' on' : ''}"
          data-k="${esc(v.key)}">${esc(v.key || '—')}<span>${evFmtInt(v.count)}</span></button>`).join('')}
      </div>
    </div>`;
  wrap.innerHTML =
    mk('suite', bank.suites, EV.banks.suite, 'suite') +
    mk('group', bank.groups.slice(0, 26), EV.banks.group, 'group') +
    mk('difficulty', bank.difficulties, EV.banks.difficulty, 'difficulty') +
    (bank.languages.length
      ? mk('language', bank.languages.slice(0, 20), EV.banks.language, 'language')
      : '');

  wrap.querySelectorAll('.ev-facet-v').forEach(row => {
    row.onclick = e => {
      const chip = e.target.closest('.ev-chip');
      if (!chip) return;
      const attr = row.dataset.attr;
      EV.banks[attr] = chip.dataset.k;
      EV.banks.page = 1;
      row.querySelectorAll('.ev-chip').forEach(c =>
        c.classList.toggle('on', c === chip));
      evLoadBankPage();
    };
  });
}

/* Fetch one page of questions for the selected bank. */
async function evLoadBankPage() {
  const body = $('#evBankBody');
  if (!body) return;
  body.innerHTML = `<tr><td colspan="7"><div class="sk-line" style="margin:8px 0"></div></td></tr>`;
  const p = new URLSearchParams({
    page: EV.banks.page, per_page: EV.banks.perPage,
  });
  if (EV.banks.q) p.set('q', EV.banks.q);
  for (const k of ['suite', 'group', 'difficulty', 'language']) {
    if (EV.banks[k] && EV.banks[k] !== 'all') p.set(k, EV.banks[k]);
  }
  let page;
  try {
    page = await api('/api/eval/banks/' + encodeURIComponent(EV.banks.sel) + '/page?' + p);
  } catch (e) {
    body.innerHTML = `<tr><td colspan="7" style="color:var(--red)">${esc(e.message)}</td></tr>`;
    return;
  }
  Object.assign(EV.banks, { pages: page.pages, total: page.total });

  if (!page.items.length) {
    body.innerHTML = `<tr><td colspan="7">${evEmpty('No questions match',
      'Adjust the search text or clear a facet filter.')}</td></tr>`;
  } else {
    body.innerHTML = page.items.map(q => `
      <tr data-id="${esc(q.id)}" title="Click for full question detail">
        <td class="mono dim" style="max-width:190px;overflow:hidden;text-overflow:ellipsis">${esc(q.id)}</td>
        <td class="mono dim">${esc(q.group || '—')}</td>
        <td class="mono dim" style="max-width:170px;overflow:hidden;text-overflow:ellipsis">${esc(q.subject || '—')}</td>
        <td class="mono dim" style="max-width:170px;overflow:hidden;text-overflow:ellipsis">${esc(q.topic || '—')}</td>
        <td>${q.difficulty ? `<span class="pill inc">${esc(q.difficulty)}</span>` : '<span class="faint">—</span>'}</td>
        <td class="mono faint" style="max-width:120px;overflow:hidden;text-overflow:ellipsis">${esc(q.questionType || '—')}</td>
        <td style="max-width:420px">
          <div style="overflow:hidden;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical">${esc(q.question)}</div>
          ${q.hasReference ? '<span class="pill ok" style="margin-top:4px;display:inline-block">answer key</span>' : ''}
        </td>
      </tr>`).join('');
    body.querySelectorAll('tr[data-id]').forEach(tr => {
      tr.onclick = () => {
        const q = page.items.find(x => x.id === tr.dataset.id);
        if (q) evQuestionDetail(q);
      };
    });
  }

  $('#evBankCount').textContent =
    `showing ${evFmtInt((page.page - 1) * page.perPage + 1)}–` +
    `${evFmtInt(Math.min(page.page * page.perPage, page.total))} of ${evFmtInt(page.total)}` +
    (page.truncated ? '' : '');
  renderPager($('#evBankPager'), page, pg => { EV.banks.page = pg; evLoadBankPage(); });
}

/* Full-screen detail for one question. */
function evQuestionDetail(q) {
  $('#pageTitle').textContent = 'Question';
  $('#pageSub').textContent = `${q.id} · ${q.group || '—'}`;
  const kv = (k, v) => v ? `<div class="dm"><div class="dm-k">${k}</div>
    <div class="dm-v">${esc(v)}</div></div>` : '';
  const extras = Object.entries(q.extra || {}).map(([k, v]) =>
    `<div class="dm"><div class="dm-k">${esc(k)}</div>
     <div class="dm-v" style="white-space:pre-wrap">${esc(String(v).slice(0, 2000))}</div></div>`).join('');

  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <button class="adv-toggle" id="evBackToBank" style="margin-bottom:12px">← back to bank</button>
    <div class="detail-grid">
      <div class="panel"><div class="panel-h"><div>
          <div class="panel-t">metadata</div>
          <div class="panel-sub">from the bank row</div></div></div>
        <div style="padding:16px 18px">
          ${kv('stable id', q.id)}
          ${kv('suite', q.suite)}
          ${kv('group', q.group)}
          ${kv('subject', q.subject)}
          ${kv('topic', q.topic)}
          ${kv('difficulty', q.difficulty)}
          ${kv('language', q.language)}
          ${kv('question type', q.questionType)}
          ${kv('options', q.hasOptions ? 'yes' : '')}
          ${kv('qhash', q.qhash)}
          ${extras}
        </div></div>
      <div class="panel"><div class="panel-h"><div>
          <div class="panel-t">question</div>
          <div class="panel-sub">${q.hasReference ? 'answer key attached' : 'no answer key in the bank'}</div>
        </div></div>
        <div style="padding:18px 20px">
          <div style="font-size:14.5px;line-height:1.7;white-space:pre-wrap">${esc(q.question)}</div>
          ${q.reference ? `
            <div style="margin-top:18px;padding-top:16px;border-top:1px solid var(--line)">
              <div class="dm-k" style="margin-bottom:6px">reference / answer key</div>
              <div class="mono" style="font-size:12px;color:var(--ink-dim);white-space:pre-wrap;line-height:1.7">${esc(q.reference)}</div>
            </div>` : ''}
        </div></div>
    </div>
  </div></div>`;
  $('#evBackToBank').onclick = () => { location.hash = '#evalbank'; renderEvalBank(); };
}

/* ============================================================
   2 · EVALUATION DATASETS  (route: evaldata)
   Every dataset-shaped resource in the kit: size, format, rows,
   live preview (JSONL/CSV/TSV columns, Parquet schema).
   ============================================================ */
async function renderEvalData() {
  $('#pageTitle').textContent = 'Evaluation Datasets';
  $('#pageSub').textContent = 'datasets in the evaluation kit · sizes, formats, record counts';

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="sk-line"></div><div class="sk-line short"></div></div></div>`;

  let files, overview;
  try {
    [files, overview] = await Promise.all([
      api('/api/eval/files?workspace=datasets'),
      evOverview(),
    ]);
  } catch (e) { return errorView(e); }

  const items = files.items || [];
  if (!items.length) {
    root.innerHTML = `<div class="content"><div class="content-inner">
      ${files.integration === 'not_configured'
        ? evNotConfigured('No evaluation kit was found on this machine.')
        : `<div class="panel"><div class="panel-h"><div>
            <div class="panel-t">Evaluation datasets</div>
            <div class="panel-sub">no row files outside the question banks</div></div></div>
          <div style="padding:20px">${evEmpty('No datasets',
            'Row files (JSONL / CSV / TSV / Parquet) that are not question banks appear here — ' +
            'for example cached comparison answers under compare_cache/.')}</div></div>`}
      </div></div>`;
    return;
  }

  const totalRows = items.reduce((a, r) => a + (r.rows || 0), 0);
  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="kpis">
      <div class="kpi"><div class="kpi-k">datasets</div>
        <div class="kpi-v">${evFmtInt(items.length)}</div>
        <div class="kpi-s">row files in the kit</div></div>
      <div class="kpi"><div class="kpi-k">total size</div>
        <div class="kpi-v">${evFmtBytes(items.reduce((a, r) => a + (r.bytes || 0), 0))}</div>
        <div class="kpi-s">on disk, unchanged</div></div>
      <div class="kpi"><div class="kpi-k">records counted</div>
        <div class="kpi-v">${evFmtInt(totalRows)}</div>
        <div class="kpi-s">non-empty lines where cheaply countable</div></div>
      <div class="kpi"><div class="kpi-k">formats</div>
        <div class="kpi-v">${new Set(items.map(r => r.kind)).size}</div>
        <div class="kpi-s">${[...new Set(items.map(r => r.kind))].join(' · ')}</div></div>
    </div>

    <div class="panel">
      <div class="panel-h"><div>
        <div class="panel-t">Datasets</div>
        <div class="panel-sub">click a row to preview its contents</div></div></div>
      <div class="table-scroll"><table>
        <thead><tr><th>file</th><th>format</th><th>records</th><th>size</th><th>modified</th></tr></thead>
        <tbody>${items.map(r => `
          <tr data-id="${esc(r.id)}">
            <td class="mono dim">${esc(r.relPath)}</td>
            <td><span class="pill inc">${esc(r.kind)}</span></td>
            <td class="mono dim">${r.rows != null ? evFmtInt(r.rows) : '<span class="faint">metadata only</span>'}</td>
            <td class="mono dim">${evFmtBytes(r.bytes)}</td>
            <td class="mono faint">${evAgo(r.modifiedMs)}</td>
          </tr>`).join('')}</tbody>
      </table></div>
    </div>
    <div class="pa-note-line">Previews are bounded: at most 50 rows or 256 KB are ever read into
      memory. Parquet files are inspected through their footer metadata only — column data is
      never decoded in the browser or the backend.</div>
  </div></div>`;

  root.querySelectorAll('tr[data-id]').forEach(tr => {
    tr.onclick = () => {
      const r = items.find(x => x.id === tr.dataset.id);
      if (r) evFilePreview(r);
    };
  });
}

/* Bounded preview of any catalogued file (shared by datasets + tools). */
async function evFilePreview(res) {
  $('#pageTitle').textContent = res.name;
  $('#pageSub').textContent = `${res.relPath} · ${evFmtBytes(res.bytes)}`;

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="sk-line"></div><div class="sk-line short"></div></div></div>`;

  let pv;
  try {
    pv = await api('/api/eval/file/' + encodeURIComponent(res.id));
  } catch (e) { return errorView(e); }

  let body;
  if (pv.mode === 'rows') {
    const cols = pv.columns.length ? pv.columns : ['value'];
    body = `<div class="table-scroll"><table>
      <thead><tr>${cols.map(c => `<th>${esc(c)}</th>`).join('')}</tr></thead>
      <tbody>${pv.rows.map(r => `<tr>
        ${cols.map((_, i) => `<td class="mono dim" style="max-width:340px;overflow:hidden;text-overflow:ellipsis"
          title="${esc(r[i] || '')}">${esc((r[i] || '').slice(0, 200))}</td>`).join('')}
      </tr>`).join('')}</tbody></table></div>`;
  } else {
    body = `<pre class="ev-pre"><code>${esc(pv.text || '')}</code></pre>`;
  }

  root.innerHTML = `<div class="content"><div class="content-inner">
    <button class="adv-toggle" id="evPvBack" style="margin-bottom:12px">← back</button>
    <div class="panel">
      <div class="panel-h"><div>
        <div class="panel-t">${esc(pv.name)}</div>
        <div class="panel-sub">${esc(pv.note)}</div></div>
        <div style="margin-left:auto;display:flex;gap:8px;align-items:center">
          <span class="pill inc">${esc(pv.kind)}</span>
          ${pv.totalRows != null ? `<span class="pill inc">${evFmtInt(pv.totalRows)} rows</span>` : ''}
          ${pv.truncated ? '<span class="pill warn">truncated preview</span>' : ''}
        </div></div>
      ${body}
    </div>
  </div></div>`;
  // Same-hash navigation fires no hashchange, so re-render explicitly.
  $('#evPvBack').onclick = () => {
    const h = (location.hash || '').replace(/^#/, '');
    if (h.startsWith('evaldata')) return renderEvalData();
    if (h.startsWith('evalreport')) return renderEvalReport();
    if (h.startsWith('evaltools')) return renderEvalTools();
    history.back();
  };
}

/* ============================================================
   3 · BENCHMARK CENTER  (route: evalbench)
   Benchmark presets (the --input banks), decoding settings and
   the comparison-model registry, all derived from real files.
   ============================================================ */
async function renderEvalBench() {
  $('#pageTitle').textContent = 'Benchmark Center';
  $('#pageSub').textContent = 'benchmark presets · decoding settings · comparison models';

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="sk-line"></div><div class="sk-line short"></div></div></div>`;

  let b, docs;
  try {
    [b, docs] = await Promise.all([api('/api/eval/benchmarks'), api('/api/eval/docs')]);
  } catch (e) { return errorView(e); }

  if (b.integration === 'not_configured') {
    root.innerHTML = `<div class="content"><div class="content-inner">
      ${evNotConfigured('The benchmark definitions come from the evaluation kit\'s bank files and run configurations.')}
    </div></div>`;
    return;
  }

  const settingLabel = s => {
    if (s.effort) return `${s.effort} @${s.temperature ?? '?'}`;
    return s.temperature === 0 ? 'greedy' : `default @${s.temperature ?? '?'}`;
  };

  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="kpis">
      <div class="kpi"><div class="kpi-k">benchmark presets</div>
        <div class="kpi-v">${evFmtInt(b.presets.length)}</div>
        <div class="kpi-s">selectable via --input</div></div>
      <div class="kpi"><div class="kpi-k">questions covered</div>
        <div class="kpi-v">${evFmtInt(b.presets.reduce((a, p) => a + p.questions, 0))}</div>
        <div class="kpi-s">sum across presets (banks overlap by design)</div></div>
      <div class="kpi"><div class="kpi-k">decoding settings used</div>
        <div class="kpi-v">${evFmtInt(b.settings.length)}</div>
        <div class="kpi-s">across discovered runs</div></div>
      <div class="kpi"><div class="kpi-k">comparison models</div>
        <div class="kpi-v">${evFmtInt(b.comparisonModels.length)}</div>
        <div class="kpi-s">registered in run configs</div></div>
    </div>

    <div class="panel">
      <div class="panel-h"><div>
        <div class="panel-t">Benchmark presets</div>
        <div class="panel-sub">each preset is a real bank file · questions counted from the file</div>
      </div></div>
      <div class="table-scroll"><table>
        <thead><tr><th>preset</th><th>bank file</th><th>questions</th>
          <th>suites</th><th>groups</th><th>size</th></tr></thead>
        <tbody>${b.presets.map(p => `
          <tr>
            <td class="mono" style="color:var(--acc)">--input ${esc(p.name)}</td>
            <td class="mono dim">${esc(p.file)}</td>
            <td class="mono dim">${evFmtInt(p.questions)}</td>
            <td class="mono faint">${p.suites.map(s => `${esc(s.key)} ${evFmtInt(s.count)}`).join(' · ')}</td>
            <td class="mono dim">${evFmtInt(p.groups)}</td>
            <td class="mono dim">${evFmtBytes(p.bytes)}</td>
          </tr>`).join('')}</tbody>
      </table></div>
    </div>

    <div class="ev-two-col">
      <div class="panel">
        <div class="panel-h"><div>
          <div class="panel-t">Decoding settings</div>
          <div class="panel-sub">temperature × reasoning effort, as configured in real runs</div>
        </div></div>
        ${b.settings.length ? `<div class="table-scroll"><table>
          <thead><tr><th>setting</th><th>temperature</th><th>effort</th><th>runs</th></tr></thead>
          <tbody>${b.settings.map(s => `
            <tr><td class="mono" style="color:var(--acc)">${esc(s.name)}</td>
              <td class="mono dim">${s.temperature != null ? s.temperature : '—'}</td>
              <td class="mono dim">${esc(s.effort || 'default')}</td>
              <td class="mono dim">${evFmtInt(s.runs)}</td></tr>`).join('')}
          </tbody></table></div>`
          : `<div style="padding:20px">${evEmpty('No runs discovered',
              'Settings appear here once a run folder with a config.json exists under internal_eval/runs/.')}</div>`}
      </div>

      <div class="panel">
        <div class="panel-h"><div>
          <div class="panel-t">Comparison models</div>
          <div class="panel-sub">answers judged blind alongside the model under test</div>
        </div></div>
        ${b.comparisonModels.length ? `<div class="table-scroll"><table>
          <thead><tr><th>model key</th><th>runs</th></tr></thead>
          <tbody>${b.comparisonModels.map(m => `
            <tr><td class="mono" style="color:var(--acc)">${esc(m.key)}</td>
              <td class="mono dim">${evFmtInt(m.runs)}</td></tr>`).join('')}
          </tbody></table></div>`
          : `<div style="padding:20px">${evEmpty('No comparison models recorded',
              'Run configurations list the comparison models they judged against.')}</div>`}
      </div>
    </div>

    ${(docs.items || []).length ? `
    <div class="panel">
      <div class="panel-h"><div>
        <div class="panel-t">How scoring works</div>
        <div class="panel-sub">from the kit's own documentation</div></div></div>
      <div style="padding:18px 20px" class="ev-doc-links">
        ${(docs.items).map(d => `
          <button type="button" class="ev-doc-link" data-id="${esc(d.id)}">
            <span class="ev-doc-link-t">${esc(d.title)}</span>
            <span class="ev-doc-link-s">${esc(d.relPath)} · ${evFmtBytes(d.bytes)}</span>
          </button>`).join('')}
      </div>
    </div>` : ''}
  </div></div>`;

  root.querySelectorAll('.ev-doc-link').forEach(btn => {
    btn.onclick = async () => {
      const id = btn.dataset.id;
      try {
        const doc = await api('/api/eval/docs/' + encodeURIComponent(id));
        evDocView(doc);
      } catch (e) { toast(e.message); }
    };
  });
}

/* Render one Markdown document (safe subset, same approach as chat). */
function evDocView(doc) {
  $('#pageTitle').textContent = doc.title;
  $('#pageSub').textContent = `${doc.relPath} · ${evFmtBytes(doc.bytes)}`;
  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <button class="adv-toggle" id="evDocBack" style="margin-bottom:12px">← back</button>
    <div class="panel">
      <div class="panel-h"><div>
        <div class="panel-t">${esc(doc.title)}</div>
        <div class="panel-sub">${esc(doc.relPath)}</div></div></div>
      <div class="ev-md">${mdToHtml(doc.markdown)}</div>
    </div>
  </div></div>`;
  // Going "back" must actually re-render even when the hash is unchanged
  // (assigning the same hash fires no hashchange event), so call the
  // owning renderer directly.
  $('#evDocBack').onclick = () => {
    const h = (location.hash || '').replace(/^#/, '');
    if (h.startsWith('evaltools')) return renderEvalTools();
    if (h.startsWith('evalbench')) return renderEvalBench();
    history.back();
  };
}

/* ============================================================
   4 · MODEL EVALUATION  (route: evalmodel)
   Discovered runs with their settings, and the cross-run /
   cross-model comparison table built from real report.json files.
   ============================================================ */
async function renderEvalModel() {
  $('#pageTitle').textContent = 'Model Evaluation';
  $('#pageSub').textContent = 'evaluation runs · settings · model comparison';

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="sk-line"></div><div class="sk-line short"></div></div></div>`;

  let runs, cmp;
  try {
    [runs, cmp] = await Promise.all([api('/api/eval/runs'), api('/api/eval/comparison')]);
  } catch (e) { return errorView(e); }

  if (runs.integration === 'not_configured') {
    root.innerHTML = `<div class="content"><div class="content-inner">
      ${evNotConfigured('Evaluation results come from run folders under internal_eval/runs/.')}
    </div></div>`;
    return;
  }

  const items = runs.items || [];
  if (!items.length) {
    root.innerHTML = `<div class="content"><div class="content-inner">
      <div class="panel"><div class="panel-h"><div>
        <div class="panel-t">Model evaluation</div>
        <div class="panel-sub">no run folders discovered</div></div></div>
        <div style="padding:20px">${evEmpty('No evaluation runs yet',
          'Each run of ./submit.sh writes a folder under internal_eval/runs/ containing ' +
          'config.json, per-setting answers and judge grades. Those folders appear here verbatim.')}</div>
      </div></div></div>`;
    return;
  }

  const scored = items.filter(r => r.reports.some(x => x.hasReport));
  const best = (cmp.rows || []).filter(r => r.isUnderTest && r.acc != null)
    .sort((a, b) => b.acc - a.acc)[0];

  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="kpis">
      <div class="kpi"><div class="kpi-k">runs discovered</div>
        <div class="kpi-v">${evFmtInt(items.length)}</div>
        <div class="kpi-s">${scored.length} with graded reports</div></div>
      <div class="kpi"><div class="kpi-k">settings evaluated</div>
        <div class="kpi-v">${evFmtInt(items.reduce((a, r) => a + r.reports.filter(x => x.hasReport).length, 0))}</div>
        <div class="kpi-s">report.json files found</div></div>
      <div class="kpi"><div class="kpi-k">best measured accuracy</div>
        <div class="kpi-v">${best ? evFmtPct(best.acc) : '—'}</div>
        <div class="kpi-s">${best ? `${esc(best.label || best.runId)} · ${esc(best.setting)}` : 'no graded settings yet'}</div></div>
      <div class="kpi"><div class="kpi-k">models compared</div>
        <div class="kpi-v">${new Set((cmp.rows || []).map(r => r.model)).size}</div>
        <div class="kpi-s">under test + comparison models</div></div>
    </div>

    <div class="panel">
      <div class="panel-h"><div>
        <div class="panel-t">Runs</div>
        <div class="panel-sub">each row is a real run folder · click for detail</div></div></div>
      <div class="table-scroll"><table>
        <thead><tr><th>run</th><th>model under test</th><th>input</th>
          <th>settings</th><th>questions</th><th>best accuracy</th><th>modified</th></tr></thead>
        <tbody>${items.map(r => {
          const accs = r.reports.flatMap(x => x.overall)
            .filter(s => s.model === 'eka' && s.acc != null).map(s => s.acc);
          const bestAcc = accs.length ? Math.max(...accs) : null;
          return `<tr data-id="${esc(r.id)}">
            <td class="mono" style="color:var(--acc)">${esc(r.label || r.id)}</td>
            <td class="mono dim">${esc(r.model || '—')}</td>
            <td class="mono dim">${esc(r.input || '—')}</td>
            <td class="mono dim">${r.reports.length}</td>
            <td class="mono dim">${r.questions != null ? evFmtInt(r.questions) : '—'}</td>
            <td class="mono dim">${bestAcc != null ? evFmtPct(bestAcc) : '<span class="faint">not graded</span>'}</td>
            <td class="mono faint">${evAgo(r.modifiedMs)}</td>
          </tr>`;}).join('')}</tbody>
      </table></div>
    </div>

    ${(cmp.rows || []).length ? `
    <div class="panel">
      <div class="panel-h"><div>
        <div class="panel-t">Model comparison</div>
        <div class="panel-sub">accuracy = (correct + ½·partial) / graded · from report.json files</div>
      </div></div>
      <div class="table-scroll"><table>
        <thead><tr><th>run</th><th>setting</th><th>model</th><th>n</th>
          <th>accuracy</th><th>fully correct</th><th>lenient</th><th>mean score</th></tr></thead>
        <tbody>${cmp.rows.map(r => `
          <tr>
            <td class="mono dim">${esc(r.runId)}</td>
            <td class="mono dim">${esc(r.setting)}</td>
            <td class="mono" style="${r.isUnderTest ? 'color:var(--acc)' : ''}">${esc(r.model)}</td>
            <td class="mono dim">${r.n != null ? evFmtInt(r.n) : '—'}</td>
            <td class="mono dim"><strong>${evFmtPct(r.acc)}</strong></td>
            <td class="mono dim">${evFmtPct(r.strict)}</td>
            <td class="mono dim">${evFmtPct(r.lenient)}</td>
            <td class="mono dim">${r.score != null ? r.score.toFixed(2) : '—'}</td>
          </tr>`).join('')}</tbody>
      </table></div>
    </div>` : ''}
  </div></div>`;

  root.querySelectorAll('tr[data-id]').forEach(tr => {
    tr.onclick = () => {
      const r = items.find(x => x.id === tr.dataset.id);
      if (r) evRunDetail(r);
    };
  });
}

/* One run in full: config, settings, per-group accuracies, errors. */
function evRunDetail(run) {
  $('#pageTitle').textContent = run.label || run.id;
  $('#pageSub').textContent = `${run.model || 'unknown model'} · ${run.reports.length} setting(s)`;

  const settingBlock = run.reports.map(rep => {
    const eka = rep.overall.find(s => s.model === 'eka');
    const others = rep.overall.filter(s => s.model !== 'eka');
    const groups = rep.by_group.slice(0, 40);
    const errTypes = rep.errors.eka
      ? Object.entries(rep.errors.eka).sort((a, b) => b[1] - a[1]).slice(0, 10) : [];
    return `<div class="panel">
      <div class="panel-h"><div>
        <div class="panel-t">${esc(rep.name)} ${rep.hasReport ? '' : '<span class="pill warn">no report</span>'}</div>
        <div class="panel-sub">temp ${rep.temperature ?? '—'} · effort ${esc(rep.effort || 'default')}
          ${rep.n != null ? ' · ' + evFmtInt(rep.n) + ' questions' : ''}</div></div>
        ${eka && eka.acc != null ? `<div style="margin-left:auto" class="ev-big-acc">${evFmtPct(eka.acc)}</div>` : ''}
      </div>
      ${rep.hasReport ? `
      <div class="ev-run-body">
        <div class="table-scroll"><table>
          <thead><tr><th>model</th><th>n</th><th>accuracy</th><th>fully correct</th>
            <th>lenient</th><th>mean score</th></tr></thead>
          <tbody>${rep.overall.map(s => `
            <tr><td class="mono" style="${s.model === 'eka' ? 'color:var(--acc)' : ''}">${esc(s.model)}</td>
              <td class="mono dim">${s.n != null ? evFmtInt(s.n) : '—'}</td>
              <td class="mono dim"><strong>${evFmtPct(s.acc)}</strong></td>
              <td class="mono dim">${evFmtPct(s.strict)}</td>
              <td class="mono dim">${evFmtPct(s.lenient)}</td>
              <td class="mono dim">${s.score != null ? s.score.toFixed(2) : '—'}</td></tr>`).join('')}
          </tbody></table></div>

        ${groups.length ? `
        <div class="ev-sub-h">accuracy by group</div>
        <div class="table-scroll"><table>
          <thead><tr><th>group</th><th>n</th><th>this model</th><th>best comparison</th><th>gap</th></tr></thead>
          <tbody>${groups.map(g => {
            const gap = (g.bestAcc != null && g.acc != null) ? g.bestAcc - g.acc : null;
            return `<tr><td class="mono dim">${esc(g.group)}</td>
              <td class="mono dim">${g.n != null ? evFmtInt(g.n) : '—'}</td>
              <td class="mono dim">${evFmtPct(g.acc)}</td>
              <td class="mono dim">${evFmtPct(g.bestAcc)}${g.bestModel ? ` <span class="faint">(${esc(g.bestModel)})</span>` : ''}</td>
              <td class="mono" style="${gap != null && gap > 0 ? 'color:var(--red)' : (gap != null && gap < 0 ? 'color:var(--acc)' : '')}">
                ${gap != null ? (gap > 0 ? '+' : '') + gap.toFixed(1) : '—'}</td></tr>`;}).join('')}
          </tbody></table></div>` : ''}

        ${errTypes.length ? `
        <div class="ev-sub-h">error types (model under test)</div>
        <div class="ev-chip-row">${errTypes.map(([t, c]) =>
          `<span class="ev-chip static">${esc(t)}<span>${evFmtInt(c)}</span></span>`).join('')}</div>` : ''}

        <div class="ev-chip-row" style="margin-top:14px">
          ${rep.gapQuestions != null ? `<span class="ev-stat">gap questions <b>${evFmtInt(rep.gapQuestions)}</b></span>` : ''}
          ${rep.onlyRight != null ? `<span class="ev-stat">only-this-model right <b>${evFmtInt(rep.onlyRight)}</b></span>` : ''}
        </div>
      </div>` : `<div style="padding:20px">${evEmpty('No report for this setting',
        'report.json is written by the final stage of the pipeline.')}</div>`}
    </div>`;
  }).join('');

  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <button class="adv-toggle" id="evRunBack" style="margin-bottom:12px">← all runs</button>
    <div class="panel">
      <div class="panel-h"><div>
        <div class="panel-t">run configuration</div>
        <div class="panel-sub">from ${esc(run.id)}/config.json</div></div></div>
      <div style="padding:16px 18px;display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:12px">
        <div class="dm"><div class="dm-k">model under test</div><div class="dm-v">${esc(run.model || '—')}</div></div>
        <div class="dm"><div class="dm-k">label</div><div class="dm-v">${esc(run.label || '—')}</div></div>
        <div class="dm"><div class="dm-k">input</div><div class="dm-v">${esc(run.input || '—')}</div></div>
        <div class="dm"><div class="dm-k">questions</div><div class="dm-v">${run.questions != null ? evFmtInt(run.questions) : '—'}</div></div>
        <div class="dm"><div class="dm-k">max tokens</div><div class="dm-v">${run.maxTokens != null ? evFmtInt(run.maxTokens) : '—'}</div></div>
        <div class="dm"><div class="dm-k">workers / setting</div><div class="dm-v">${run.workers != null ? evFmtInt(run.workers) : '—'}</div></div>
        <div class="dm"><div class="dm-k">server urls</div><div class="dm-v">${esc((run.urls || []).join(', ') || '—')}</div></div>
        <div class="dm"><div class="dm-k">comparison models</div><div class="dm-v">${esc((run.compare || []).join(', ') || '—')}</div></div>
      </div>
    </div>
    ${settingBlock}
  </div></div>`;
  $('#evRunBack').onclick = () => { location.hash = '#evalmodel'; renderEvalModel(); };
}

/* ============================================================
   5 · REPORTS & ANALYTICS  (route: evalreport)
   Report artifacts (summary.html/json, report.json/html,
   graded_flat.csv) plus the documentation set.
   ============================================================ */
async function renderEvalReport() {
  $('#pageTitle').textContent = 'Reports & Analytics';
  $('#pageSub').textContent = 'evaluation reports · summaries · graded data';

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="sk-line"></div><div class="sk-line short"></div></div></div>`;

  let files, runs, overview;
  try {
    [files, runs, overview] = await Promise.all([
      api('/api/eval/files?workspace=reports'),
      api('/api/eval/runs'),
      evOverview(),
    ]);
  } catch (e) { return errorView(e); }

  const reports = files.items || [];
  const runItems = runs.items || [];
  const summaries = runItems.filter(r => r.summary);

  if (!reports.length && !summaries.length) {
    root.innerHTML = `<div class="content"><div class="content-inner">
      ${files.integration === 'not_configured'
        ? evNotConfigured('Reports are produced by the final stage of each evaluation run.')
        : `<div class="panel"><div class="panel-h"><div>
            <div class="panel-t">Reports & analytics</div>
            <div class="panel-sub">no report artifacts discovered</div></div></div>
          <div style="padding:20px">${evEmpty('No reports yet',
            'The pipeline writes summary.html/json per run and report.html/json + graded_flat.csv per setting.')}</div></div>`}
    </div></div>`;
    return;
  }

  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="kpis">
      <div class="kpi"><div class="kpi-k">report artifacts</div>
        <div class="kpi-v">${evFmtInt(reports.length)}</div>
        <div class="kpi-s">summary/report files on disk</div></div>
      <div class="kpi"><div class="kpi-k">runs with summary</div>
        <div class="kpi-v">${evFmtInt(summaries.length)}</div>
        <div class="kpi-s">summary.json parsed</div></div>
      <div class="kpi"><div class="kpi-k">graded settings</div>
        <div class="kpi-v">${evFmtInt(runItems.reduce((a, r) => a + r.reports.filter(x => x.hasReport).length, 0))}</div>
        <div class="kpi-s">report.json files</div></div>
      <div class="kpi"><div class="kpi-k">total report bytes</div>
        <div class="kpi-v">${evFmtBytes(reports.reduce((a, r) => a + r.bytes, 0))}</div>
        <div class="kpi-s">unchanged on disk</div></div>
    </div>

    ${reports.length ? `
    <div class="panel">
      <div class="panel-h"><div>
        <div class="panel-t">Report artifacts</div>
        <div class="panel-sub">click to preview JSON / CSV contents</div></div></div>
      <div class="table-scroll"><table>
        <thead><tr><th>artifact</th><th>format</th><th>size</th><th>modified</th></tr></thead>
        <tbody>${reports.map(r => `
          <tr data-id="${esc(r.id)}">
            <td class="mono dim">${esc(r.relPath)}</td>
            <td><span class="pill inc">${esc(r.kind)}</span></td>
            <td class="mono dim">${evFmtBytes(r.bytes)}</td>
            <td class="mono faint">${evAgo(r.modifiedMs)}</td>
          </tr>`).join('')}</tbody>
      </table></div>
    </div>` : ''}

    ${summaries.length ? `
    <div class="panel">
      <div class="panel-h"><div>
        <div class="panel-t">Run summaries</div>
        <div class="panel-sub">headline numbers from each run's summary.json</div></div></div>
      <div style="padding:16px 18px;display:grid;gap:12px">
        ${summaries.map(r => {
          const s = r.summary || {};
          const best = (r.reports || [])
            .flatMap(x => x.overall || [])
            .filter(o => o.model === 'eka' && o.acc != null)
            .sort((a, b) => b.acc - a.acc)[0];
          return `<div class="ev-sum-row">
            <div>
              <div class="ev-sum-t">${esc(r.label || r.id)}</div>
              <div class="ev-sum-s">${esc(r.id)} · ${r.questions != null ? evFmtInt(r.questions) + ' questions' : '—'}
                ${s.n_questions != null ? ' · summary n=' + evFmtInt(s.n_questions) : ''}</div>
            </div>
            <div class="ev-sum-acc">${best ? evFmtPct(best.acc) : '—'}</div>
          </div>`;}).join('')}
      </div>
    </div>` : ''}
  </div></div>`;

  root.querySelectorAll('tr[data-id]').forEach(tr => {
    tr.onclick = () => {
      const r = reports.find(x => x.id === tr.dataset.id);
      if (r) evFilePreview(r);
    };
  });
}

/* ============================================================
   6 · EVALUATION TOOLS  (route: evaltools)
   Scripts, Slurm jobs, configs, tests and documentation —
   the operational side of the kit, browsable and previewable.
   ============================================================ */
async function renderEvalTools() {
  $('#pageTitle').textContent = 'Evaluation Tools';
  $('#pageSub').textContent = 'pipeline stages · slurm jobs · configs · tests · documentation';

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="sk-line"></div><div class="sk-line short"></div></div></div>`;

  let files, docs;
  try {
    [files, docs] = await Promise.all([
      api('/api/eval/files?workspace=tools'),
      api('/api/eval/docs'),
    ]);
  } catch (e) { return errorView(e); }

  if (files.integration === 'not_configured') {
    root.innerHTML = `<div class="content"><div class="content-inner">
      ${evNotConfigured('The tooling lives inside the evaluation kit (pipeline/, slurm/, tests/, config/).')}
    </div></div>`;
    return;
  }

  const items = files.items || [];
  const byDir = new Map();
  for (const r of items) {
    const d = r.dir || '.';
    if (!byDir.has(d)) byDir.set(d, []);
    byDir.get(d).push(r);
  }

  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="kpis">
      <div class="kpi"><div class="kpi-k">resources</div>
        <div class="kpi-v">${evFmtInt(items.length)}</div>
        <div class="kpi-s">scripts, configs, tests, docs</div></div>
      <div class="kpi"><div class="kpi-k">python stages</div>
        <div class="kpi-v">${evFmtInt(items.filter(r => r.kind === 'Python').length)}</div>
        <div class="kpi-s">pipeline/*.py</div></div>
      <div class="kpi"><div class="kpi-k">slurm jobs</div>
        <div class="kpi-v">${evFmtInt(items.filter(r => r.kind === 'Slurm').length)}</div>
        <div class="kpi-s">slurm/*.sbatch</div></div>
      <div class="kpi"><div class="kpi-k">documents</div>
        <div class="kpi-v">${evFmtInt((docs.items || []).length)}</div>
        <div class="kpi-s">markdown guides</div></div>
    </div>

    ${(docs.items || []).length ? `
    <div class="panel">
      <div class="panel-h"><div>
        <div class="panel-t">Documentation</div>
        <div class="panel-sub">the kit's own guides · rendered from markdown</div></div></div>
      <div style="padding:14px 16px;display:grid;gap:9px">
        ${docs.items.map(d => `
          <button type="button" class="ev-doc-link" data-id="${esc(d.id)}">
            <span class="ev-doc-link-t">${esc(d.title)}</span>
            <span class="ev-doc-link-s">${esc(d.relPath)} · ${evFmtBytes(d.bytes)}</span>
          </button>`).join('')}
      </div>
    </div>` : ''}

    ${[...byDir.entries()].sort().map(([dir, rs]) => `
    <div class="panel">
      <div class="panel-h"><div>
        <div class="panel-t">${esc(dir === '.' ? 'kit root' : dir + '/')}</div>
        <div class="panel-sub">${rs.length} file${rs.length === 1 ? '' : 's'} · click to preview</div>
      </div></div>
      <div class="table-scroll"><table>
        <thead><tr><th>file</th><th>kind</th><th>size</th><th>modified</th></tr></thead>
        <tbody>${rs.map(r => `
          <tr data-id="${esc(r.id)}">
            <td class="mono dim">${esc(r.name)}</td>
            <td><span class="pill inc">${esc(r.kind)}</span></td>
            <td class="mono dim">${evFmtBytes(r.bytes)}</td>
            <td class="mono faint">${evAgo(r.modifiedMs)}</td>
          </tr>`).join('')}</tbody>
      </table></div>
    </div>`).join('')}

    <div class="pa-note-line">Read-only. The authoritative runners are the kit's own
      <code>./submit.sh</code> and <code>sbatch</code> scripts — this workspace shows
      their source and configuration, it never executes them.</div>
  </div></div>`;

  root.querySelectorAll('tr[data-id]').forEach(tr => {
    tr.onclick = () => {
      const r = items.find(x => x.id === tr.dataset.id);
      if (r) evFilePreview(r);
    };
  });
  root.querySelectorAll('.ev-doc-link').forEach(btn => {
    btn.onclick = async () => {
      try {
        const doc = await api('/api/eval/docs/' + encodeURIComponent(btn.dataset.id));
        evDocView(doc);
      } catch (e) { toast(e.message); }
    };
  });
}

/* Sub-route helper for deep links (#evalbank/qbank.jsonl). */
function evSubRoute(parent) {
  const h = (location.hash || '').replace(/^#/, '');
  if (!h.startsWith(parent + '/')) return null;
  return decodeURIComponent(h.slice(parent.length + 1));
}
