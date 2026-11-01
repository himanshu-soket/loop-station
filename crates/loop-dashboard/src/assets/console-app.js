'use strict';
/* ============================================================
   loop console — inference playground + durable trace explorer
   All data from the local API. Streaming via fetch + SSE parse.
   ============================================================ */

const S = {
  route: 'overview',
  health: null,
  models: [],
  paramsMeta: null,
  conversations: [],
  activeConv: null,
  streaming: null,
  traces: { items: [], total: 0, page: 1, perPage: 25, pages: 1,
            q: '', status: 'all', sort: 'started', dir: 'desc' },
  requests: { items: [], total: 0, page: 1, perPage: 25, pages: 1,
              q: '', status: 'all' },
};

const $ = s => document.querySelector(s);
const el = (t, c, h) => { const n = document.createElement(t); if (c) n.className = c; if (h != null) n.innerHTML = h; return n; };
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const debounce = (fn, ms) => { let t; return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); }; };

/* ---------- formatting ---------- */
const fmtInt = n => (n ?? 0).toLocaleString('en-US');
const fmtMs = ms => {
  if (ms == null) return '—';
  if (ms < 1000) return Math.round(ms) + ' ms';
  if (ms < 60000) return (ms/1000).toFixed(2) + ' s';
  return Math.floor(ms/60000) + 'm ' + Math.round((ms%60000)/1000) + 's';
};
const fmtTime = ts => {
  if (!ts) return '—';
  const d = new Date(ts), diff = Date.now() - ts;
  if (diff < 60000) return 'just now';
  if (diff < 3600000) return Math.floor(diff/60000) + 'm ago';
  if (new Date().toDateString() === d.toDateString())
    return d.toLocaleTimeString('en-US',{hour:'2-digit',minute:'2-digit'});
  return d.toLocaleDateString('en-US',{month:'short',day:'numeric'});
};
const fmtTimeFull = ts => ts ? new Date(ts).toLocaleString('en-US',{month:'short',day:'numeric',hour:'2-digit',minute:'2-digit',second:'2-digit'}) : '—';

function toast(msg) {
  const t = $('#toast');
  t.textContent = msg; t.classList.remove('hidden');
  clearTimeout(toast._t);
  toast._t = setTimeout(() => t.classList.add('hidden'), 2200);
}

async function api(path, opts) {
  const res = await fetch(path, opts);
  // Session expired or never established: go re-authenticate, preserving
  // the deep link so sign-in returns the user where they were.
  if (res.status === 401) {
    const next = encodeURIComponent(location.pathname + location.hash);
    location.replace('/signin?next=' + next);
    throw new Error('authentication required');
  }
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.error || ('HTTP ' + res.status));
  return body;
}

/* ---------- session chip (shared shell) ----------
 * Shows the signed-in owner and offers sign-out. Hidden when auth is not
 * configured (standalone --no-auth mode) — /api/auth/status stays honest.
 *
 * The "Account approvals" link is added only when the SERVER says this
 * caller may use the admin API. We probe /api/admin/users/pending and show
 * the link on 200, hide it on 403/401. The link is never authorization:
 * hiding it protects UX, while every /api/admin/* request is independently
 * role-checked by require_admin() on the server. */
async function initSessionChip() {
  const chip = document.getElementById('userChip');
  if (!chip) return;
  try {
    const st = await fetch('/api/auth/status').then(r => r.json());
    if (!st.signedIn) { chip.hidden = true; return; }
    chip.hidden = false;
    const name = String(st.identifier || 'owner');
    document.getElementById('userName').textContent = name;
    document.getElementById('userAvatar').textContent =
      name.trim().charAt(0).toUpperCase() || '·';
    maybeShowAdminLink();
    const btn = document.getElementById('signOutBtn');
    if (btn) btn.onclick = async () => {
      btn.disabled = true;
      try {
        await fetch('/api/auth/signout', { method: 'POST' });
      } catch { /* network gone: cookie clearing below still helps */ }
      location.replace('/signin');
    };
  } catch { chip.hidden = true; }
}

/* Ask the server whether this caller can administer accounts, and reveal
 * the approvals link only then. Any failure (network, 401, 403) leaves the
 * link hidden — never a false positive. */
async function maybeShowAdminLink() {
  const slot = document.getElementById('adminNavLink');
  if (!slot) return;
  try {
    const res = await fetch('/api/admin/users/pending', { credentials: 'same-origin' });
    if (res.ok) slot.hidden = false;
  } catch { /* unreachable: keep hidden */ }
}

/* ---------- markdown (safe subset, escaped first) ---------- */
function mdToHtml(src) {
  if (!src) return '';
  const lines = String(src).split('\n');
  let html = '', inCode = false, codeBuf = [], listMode = null, para = [];
  const flushPara = () => { if (para.length) { html += '<p>' + inline(para.join(' ')) + '</p>'; para = []; } };
  const flushList = () => { if (listMode) { html += `</${listMode}>`; listMode = null; } };
  for (const line of lines) {
    if (/^```/.test(line.trim())) {
      if (inCode) {
        const code = codeBuf.join('\n');
        html += `<pre><code>${esc(code)}</code><button class="copy-code">copy</button></pre>`;
        inCode = false; codeBuf = [];
      } else { flushPara(); flushList(); inCode = true; }
      continue;
    }
    if (inCode) { codeBuf.push(line); continue; }
    if (!line.trim()) { flushPara(); flushList(); continue; }
    const h = line.match(/^(#{1,4})\s+(.*)/);
    if (h) { flushPara(); flushList(); const l = h[1].length; html += `<h${l}>${inline(h[2])}</h${l}>`; continue; }
    if (/^\s*[-*]\s+/.test(line)) {
      flushPara();
      if (listMode !== 'ul') { flushList(); html += '<ul>'; listMode = 'ul'; }
      html += `<li>${inline(line.replace(/^\s*[-*]\s+/, ''))}</li>`; continue;
    }
    if (/^\s*\d+\.\s+/.test(line)) {
      flushPara();
      if (listMode !== 'ol') { flushList(); html += '<ol>'; listMode = 'ol'; }
      html += `<li>${inline(line.replace(/^\s*\d+\.\s+/, ''))}</li>`; continue;
    }
    if (/^>\s?/.test(line)) { flushPara(); flushList(); html += `<blockquote>${inline(line.replace(/^>\s?/, ''))}</blockquote>`; continue; }
    flushList();
    para.push(line);
  }
  if (inCode && codeBuf.length) html += `<pre><code>${esc(codeBuf.join('\n'))}</code></pre>`;
  flushPara(); flushList();
  return html;

  function inline(s) {
    return esc(s)
      .replace(/`([^`]+)`/g, '<code>$1</code>')
      .replace(/\*\*([^*]+)\*\*/g, '<b>$1</b>')
      .replace(/\[([^\]]+)\]\((https?:[^)]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>');
  }
}

/* ---------- nav ---------- */
const ICONS = {
  overview:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M3 13a9 9 0 1 0 9-9"/><path d="M3 13V4"/><path d="M13 3v10h8"/></svg>',
  chat:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>',
  traces:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12h4l3-8 4 16 3-8h6"/></svg>',
  requests:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a15 15 0 0 1 0 18M12 3a15 15 0 0 0 0 18"/></svg>',
  models:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><circle cx="4.5" cy="6" r="2"/><circle cx="19.5" cy="6" r="2"/><circle cx="4.5" cy="18" r="2"/><circle cx="19.5" cy="18" r="2"/><path d="M6.2 7.2 9.6 10M17.8 7.2 14.4 10M6.2 16.8 9.6 14M17.8 16.8 14.4 14"/></svg>',
  analytics:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 3v18h18"/><rect x="7" y="12" width="3" height="6" rx="1"/><rect x="12" y="8" width="3" height="10" rx="1"/><rect x="17" y="5" width="3" height="13" rx="1"/></svg>',
  settings:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>',
  /* Workspace icons for the Loop portal cards. */
  system:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/></svg>',
  loop:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12h4l3-8 4 16 3-8h6"/></svg>',
  sft:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v18M5 8l7-5 7 5M5 16l7 5 7-5"/></svg>',
  pretrain:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="6" rx="8" ry="3"/><path d="M4 6v6c0 1.7 3.6 3 8 3s8-1.3 8-3V6"/><path d="M4 12v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6"/></svg>',
  /* Phase 10: Fun Time — a game controller. */
  funtime:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M6 12h4M8 10v4"/><circle cx="15.5" cy="11" r=".6"/><circle cx="17.5" cy="13" r=".6"/><rect x="2.5" y="7" width="19" height="10" rx="5"/></svg>',
  /* Internal evaluation workspaces — one icon per section, used by the
   * unified workspace's own compact sidebar. */
  eval:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/></svg>',
  evalbank:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 19.5V5a2 2 0 0 1 2-2h11a1 1 0 0 1 1 1v16"/><path d="M6 17h12v3H6a2 2 0 0 1 0-4h12"/><path d="M9 7h6"/></svg>',
  evaldata:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="5.5" rx="7.5" ry="2.8"/><path d="M4.5 5.5v13c0 1.5 3.4 2.8 7.5 2.8s7.5-1.3 7.5-2.8v-13"/><path d="M4.5 12c0 1.5 3.4 2.8 7.5 2.8s7.5-1.3 7.5-2.8"/></svg>',
  evalbench:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 20h18"/><path d="M6 20V9m6 11V4m6 16v-7"/><path d="M3 4l4 3 4-4 4 3 4-4"/></svg>',
  evalmodel:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="4" width="16" height="16" rx="3"/><path d="M9 9h6v6H9z"/><path d="M9 1v3M15 1v3M9 20v3M15 20v3M1 9h3M1 15h3M20 9h3M20 15h3"/></svg>',
  evalreport:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><path d="M14 2v6h6"/><path d="M8 13h8M8 17h5"/></svg>',
  evaltools:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14.7 6.3a4.5 4.5 0 0 0-6 6L3 18l3 3 5.7-5.7a4.5 4.5 0 0 0 6-6L14 13l-3-3z"/></svg>',
  /* Phase 11: Space Invaders — an alien ship. */
  invader:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2v3M7.5 4.5v2.5M16.5 4.5v2.5"/><rect x="4" y="7" width="16" height="10" rx="3.5"/><path d="M9 11.5h.01M15 11.5h.01"/><path d="M8.5 17v3M15.5 17v3M12 17v2"/></svg>',
};
/* Navigation: organized around the portal's four workspaces. The top group
 * is the primary navigation — the same five destinations every shell
 * offers. The second group holds the Loop AI Console's own sections, which
 * this shell hosts natively; SFT and Pretraining DataTrove are single-page
 * workspaces whose detail lives inside the page. The trace observatory
 * stays available at its direct URL but is not a primary destination. */
const NAV_GROUPS = [
  /* Primary workspaces — one entry per card on the Welcome page. */
  { label: 'Workspaces', items: [
    ['overview', 'Welcome'],
    ['/system#overview', 'System Dashboard', 'ext', 'system'],
    ['loop', 'Loop AI Console'],
    ['sft', 'SFT DataTrove'],
    ['pretrain', 'Pretraining DataTrove'],
    ['funtime', 'Fun Time'],
    ['eval', 'Internal Evaluation'],
  ] },
  /* Sections of the Loop AI Console workspace (rendered by this shell).
   * `ws` ties the group to its owning workspace so renderNav() can show a
   * workspace's own sections only while you are inside it. */
  { label: 'Loop AI Console', ws: 'loop', items: [
    ['chat', 'Playground'], ['traces', 'Traces & Requests'],
    ['models', 'Models & Providers'], ['analytics', 'Analytics'], ['settings', 'Settings'],
  ] },
];

/* Which hash routes belong to each primary workspace, so the sidebar
 * highlights the workspace that owns the current view (#traces lights
 * "Loop AI Console", not just its section entry). */
const WS_ROUTES = {
  overview: ['overview'],
  loop: ['loop', 'chat', 'traces', 'requests', 'models', 'analytics', 'settings'],
  sft: ['sft'],
  pretrain: ['pretrain'],
  funtime: ['funtime'],
  /* One unified Internal Evaluation workspace: #eval is the shell; its
   * sections (#eval/bank, #eval/data, …) all belong to it. */
  eval: ['eval', 'evalbank', 'evaldata', 'evalbench', 'evalmodel',
    'evalreport', 'evaltools'],
};
function renderNav() {
  const nav = $('#nav'); nav.innerHTML = '';
  // Which primary workspace owns the current route? ('overview' = Welcome.)
  const owner = Object.keys(WS_ROUTES).find(k => WS_ROUTES[k].includes(S.route)) || 'overview';
  for (const group of NAV_GROUPS) {
    // The Workspaces group is the global switcher and always shows; a
    // workspace's own section list appears only inside that workspace.
    if (group.ws && group.ws !== owner) continue;
    if (group.label) {
      const g = el('div', 'nav-group', group.label);
      nav.appendChild(g);
    }
    for (const item of group.items) {
      const [id, label, kind, icon] = item;
      if (kind === 'ext') {
        // External workspace: a real link so it opens in this tab with
        // proper browser history, and is keyboard focusable.
        const a = document.createElement('a');
        a.className = 'nav-item';
        a.href = id;
        a.title = label;
        a.innerHTML = (ICONS[icon || 'overview']) + `<span>${label}</span>`;
        nav.appendChild(a);
        continue;
      }
      // A primary item is active when the current route belongs to its
      // workspace; a section item when it is the current route (#loop
      // renders the chat view, so Playground stays highlighted there too).
      const active = (WS_ROUTES[id] || [id]).includes(S.route) ||
        (id === 'chat' && S.route === 'loop');
      const n = el('div', 'nav-item' + (active ? ' active' : ''),
        (ICONS[id] || ICONS.overview) + `<span>${label}</span>`);
      n.title = label;
      n.onclick = () => go(id);
      nav.appendChild(n);
    }
  }
}
function go(route) {
  S.route = route;
  location.hash = route;
  renderNav();
  render();
}

async function refreshHealth() {
  try {
    S.health = await api('/api/health');
    $('#connDot').classList.toggle('off', !S.health.ok);
    $('#connLabel').textContent = S.health.ok ? 'connected' : 'backend issue';
  } catch {
    $('#connDot').classList.add('off');
    $('#connLabel').textContent = 'offline';
  }
}

/* ============================================================
   OVERVIEW
   ============================================================ */
/* ============================================================
   SOKET WORKFLOW WELCOME PAGE (route: overview)
   The /console landing view: one entry point for the platform's two
   workspaces. Every number comes from a real endpoint; nothing is
   simulated. Each card loads independently — one failing backend
   degrades its own card, never the page.
   ============================================================ */
const fmtBytes = (n, dp = 1) => {
  if (n == null || !isFinite(n)) return '—';
  const u = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  let i = 0, v = n;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return v.toFixed(i === 0 ? 0 : dp) + ' ' + u[i];
};
const ago = ms => {
  if (!ms) return '—';
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 60) return Math.round(s) + 's ago';
  if (s < 3600) return Math.round(s / 60) + 'm ago';
  if (s < 86400) return Math.round(s / 3600) + 'h ago';
  return Math.round(s / 86400) + 'd ago';
};
/* Compact token-count formatting for the pipeline workspaces. */
const fmtTokensEst = n => {
  if (n == null || !isFinite(n)) return '—';
  if (n >= 1e12) return (n / 1e12).toFixed(2) + 'T';
  if (n >= 1e9) return (n / 1e9).toFixed(2) + 'B';
  if (n >= 1e6) return (n / 1e6).toFixed(1) + 'M';
  if (n >= 1e3) return (n / 1e3).toFixed(1) + 'K';
  return String(Math.round(n));
};

/* Workspace descriptors: id, title, accent, destination route in this SPA
 * (or an external path when the workspace lives in another shell page).
 * Availability for pipeline workspaces comes from /api/pipelines/portal —
 * never hardcoded, so a missing project reports "not integrated yet". */
const WORKSPACES = [
  { id: 'system',  title: 'System Dashboard',
    href: '/system#overview',
    blurb: 'Monitor system health, CPU, memory, GPU, and performance.',
    open: 'Open System Dashboard' },
  { id: 'loop',    title: 'Loop AI Console',
    href: '#loop',
    blurb: 'Chat with models, inspect traces, manage providers, and explore analytics.',
    open: 'Open Loop AI Console' },
  { id: 'sft',     title: 'SFT DataTrove',
    href: '#sft',
    blurb: 'Access existing supervised fine-tuning data preparation and quality-control tools.',
    open: 'Open SFT DataTrove' },
  { id: 'pretrain', title: 'Pretraining DataTrove',
    href: '#pretrain',
    blurb: 'Access existing pretraining corpus and processing tools.',
    open: 'Open Pretraining DataTrove' },
  /* Phase 10: the games destination. A hash route inside this SPA (not an
   * external page), so it inherits the session guard, the shell chrome and
   * browser history for free. The Dino Runner stays on the sign-in screen
   * only — it is deliberately NOT offered here. */
  { id: 'funtime', title: 'Fun Time',
    href: '#funtime',
    blurb: 'Take a break. Play a game.',
    open: 'Play Now' },
  /* Internal evaluation: ONE workspace card. The unified dashboard it opens
   * carries its own compact sidebar (Overview, Question Bank, Datasets,
   * Benchmarks, Model Evaluation, Reports, Tools) inside #eval — the Home
   * page no longer shows one card per section. */
  { id: 'eval', title: 'Internal Evaluation',
    href: '#eval',
    blurb: 'Benchmarks, question banks, datasets, evaluation runs and reports.',
    open: 'Open Internal Evaluation' },
];

/* One workspace card. The entire card is a single anchor so the whole
 * surface is clickable and keyboard-focusable; the foot carries the visible
 * action label. No metric rows here — the Welcome page is a clean
 * workspace-selection screen (live data lives inside each workspace). */
function portalCardSkeleton(ws) {
  return `<a class="ws-card" id="ws-${ws.id}" href="${ws.href}" title="${ws.open}"
    aria-label="${ws.title} — ${ws.blurb}">
    <div class="ws-head">
      <div class="ws-ico">${ICONS[ws.id] || ICONS.overview}</div>
      <div class="ws-title"><div class="ws-name">${ws.title}</div>
        <div class="ws-blurb">${ws.blurb}</div></div>
      <span class="ws-badge sk">…</span>
    </div>
    <div class="ws-foot"><span class="ws-open">${ws.open} →</span></div>
  </a>`;
}

/* Badge for an integration/connectivity state. `kind` picks the colour:
 * ok = healthy/connected, idle = present but nothing ran, warn = degraded,
 * na = not configured / unavailable. */
function wsBadge(text, kind) {
  return `<span class="ws-badge ${kind}">${esc(text)}</span>`;
}

async function renderOverview() {
  const root = $('#viewRoot');
  $('#pageTitle').textContent = 'Loop';
  $('#pageSub').textContent = 'unified AI development & data operations workspace';

  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="portal-hero">
      <div>
        <h1 class="portal-h1">Welcome to Loop</h1>
        <div class="portal-tag">Your unified AI development and data operations workspace</div>
        <p class="portal-sub">One place to monitor the machine your models run on, work with
          the Loop model, and prepare the data that trains it.</p>
      </div>
      <button class="portal-refresh" id="portalRefresh" title="Reload workspace summaries">
        ↻ refresh</button>
    </div>

    <div class="ws-grid" id="wsGrid">
      ${WORKSPACES.map(portalCardSkeleton).join('')}
    </div>
  </div></div>`;

  // Workspace status badges load independently; failures stay local.
  fillSystemCard();
  fillLoopCard();
  fillPipelineCards();
  fillEvalCards();

  const btn = $('#portalRefresh');
  if (btn) btn.onclick = () => renderOverview();
}

/* ---- Internal evaluation card ---------------------------------------
 * Real source: /api/eval/overview. The single Home card carries the kit's
 * headline numbers (questions, datasets, runs); every number is a real
 * count from the kit on disk. A missing kit shows an honest "not
 * configured" state. */
async function fillEvalCards() {
  const setBadge = (id, html) => {
    const card = document.getElementById('ws-' + id);
    if (card) card.querySelector('.ws-badge').outerHTML = html;
  };
  let o;
  try {
    o = await api('/api/eval/overview');
  } catch {
    setBadge('eval', wsBadge('status unknown', 'err'));
    return;
  }

  if (o.integration === 'not_configured') {
    setBadge('eval', wsBadge('not configured', 'na'));
    return;
  }

  // One badge with the workspace's own headline numbers.
  const b = o.banks || {}, d = o.datasets || {}, m = o.evaluations || {};
  const parts = [];
  if ((b.questions || 0) > 0) parts.push(`${fmtTokensEst(b.questions)} questions`);
  if ((d.files || 0) > 0) parts.push(`${d.files} dataset${d.files === 1 ? '' : 's'}`);
  if ((m.runs || 0) > 0) {
    parts.push(m.best
      ? `${m.runs} run${m.runs === 1 ? '' : 's'} · best ${m.best.acc.toFixed(1)}%`
      : `${m.runs} run${m.runs === 1 ? '' : 's'}`);
  }
  setBadge('eval',
    parts.length ? wsBadge(parts.join(' · '), 'ok') : wsBadge('connected · empty', 'idle'));
}

/* ---- Pipeline cards (SFT + Pretraining DataTrove) -------------------
 * Real source: /api/pipelines/portal. Status is whatever the scanner
 * reports — connected, idle, or not_configured ("not integrated yet").
 * Never invented: a missing project shows an honest unavailable state.
 * The Welcome card carries only the badge; run tables and setup details
 * live inside each pipeline workspace. */
async function fillPipelineCards() {
  let portal;
  try {
    portal = await api('/api/pipelines/portal');
  } catch {
    for (const id of ['sft', 'pretrain']) {
      const el = document.getElementById('ws-' + id);
      if (el) el.querySelector('.ws-badge').outerHTML = wsBadge('status unknown', 'err');
    }
    return;
  }

  const badgeFor = (p) => {
    if (p.integration === 'connected') return wsBadge('connected', 'ok');
    if (p.integration === 'idle') return wsBadge('connected · no runs', 'idle');
    return wsBadge('not integrated yet', 'na');
  };
  for (const [id, data] of [['sft', portal.sft], ['pretrain', portal.pretrain]]) {
    const el = document.getElementById('ws-' + id);
    if (el && data) el.querySelector('.ws-badge').outerHTML = badgeFor(data);
  }
}

/* ---- System Dashboard card ----------------------------------------
 * Real source: /api/sys/health — the consolidated health document computed
 * from the live sampler (status, freshness, per-domain availability).
 * Honest states only: healthy / degraded / stale / unknown map to badges;
 * unavailable domains say so instead of showing zeros. */
/* ---- System Dashboard card ----------------------------------------
 * Real source: /api/sys/health — the consolidated health document computed
 * from the live sampler. The Welcome card carries only the aggregate
 * status badge; detailed metrics live inside the System Dashboard. */
async function fillSystemCard() {
  const el = document.getElementById('ws-system');
  if (!el) return;
  let h;
  try {
    h = await api('/api/sys/health');
  } catch {
    el.querySelector('.ws-badge').outerHTML = wsBadge('unavailable', 'err');
    return;
  }
  // Aggregate status → badge. `unknown` covers both a not-yet-sampled
  // monitor and one whose status cannot be trusted.
  const badgeFor = {
    healthy:   () => wsBadge(h.demo ? 'demo data' : 'healthy', h.demo ? 'warn' : 'ok'),
    degraded:  () => wsBadge('degraded', 'warn'),
    stale:     () => wsBadge('stale data', 'warn'),
    unknown:   () => wsBadge('no samples yet', 'na'),
  };
  el.querySelector('.ws-badge').outerHTML =
    (badgeFor[h.status] || (() => wsBadge('unknown', 'na')))();
}

const fmtPctPortal = (v, dp = 1) =>
  (v == null || !isFinite(v)) ? '—' : Number(v).toFixed(dp) + '%';

/* ---- Loop AI Console card ------------------------------------------
 * Real source: /api/health (inference readiness + session DB). The Welcome
 * card carries only the connectivity badge; model details, sessions and
 * traces live inside the console workspace. */
async function fillLoopCard() {
  const el = document.getElementById('ws-loop');
  if (!el) return;
  try {
    const health = await api('/api/health');
    el.querySelector('.ws-badge').outerHTML = health.ok
      ? (health.tracesAvailable === false ? wsBadge('degraded', 'warn') : wsBadge('connected', 'ok'))
      : wsBadge('backend issue', 'err');
  } catch {
    el.querySelector('.ws-badge').outerHTML = wsBadge('disconnected', 'err');
  }
}

/* ============================================================
   SFT DATATROVE WORKSPACE (route: sft)
   Real source: /api/pipelines/sft → sftqc run reports on disk.
   Read-only. The pipeline's own CLI is the authoritative runner; the
   command to reproduce a run is shown, never executed from here.
   ============================================================ */
async function renderSftPipeline() {
  // Sub-route: the interactive workflow canvas (documented design).
  if (paSubRoute() === 'workflow') return renderSftWorkflow();
  // Sub-route: the architecture canvas transcribed from the sftqc source.
  if (paSubRoute() === 'sft-architecture') return renderSftArchitecture();
  // Sub-route: the EKA SFT data-pipeline board (embedded page).
  if (paSubRoute() === 'sft-board') return renderSftBoard();
  // Sub-route: the code explorer over the real Python files.
  if (paSubRoute() === 'sft-code') return renderSftCodeExplorer();
  swTeardown();

  $('#pageTitle').textContent = 'SFT DataTrove';
  $('#pageSub').textContent = 'supervised fine-tuning data preparation · quality-gated by sftqc';

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="sk-line"></div><div class="sk-line short"></div></div></div>`;

  let s;
  try {
    s = await api('/api/pipelines/sft');
  } catch (e) { return errorView(e); }

  if (s.integration === 'not_configured') {
    root.innerHTML = `<div class="content"><div class="content-inner">
      ${notConfiguredPanel('SFT DataTrove', s.unavailableReason, 'sftqc')}
      <div class="pa-entry">
        <div>
          <div class="pa-entry-t">EKA SFT data pipeline</div>
          <div class="pa-entry-s">The six-stage board — Collect → Standardize →
            Clean corpus → Curate gold → Audit + handoff → Train + feedback —
            with each stage's rejected rows and where they are repaired.</div>
        </div>
        <a class="pa-entry-btn" href="#sft/board">Open pipeline board
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
               stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg></a>
      </div>

      <div class="pa-entry">
        <div>
          <div class="pa-entry-t">SFT pipeline map</div>
          <div class="pa-entry-s">The original EKA SFT data-pipeline board —
            the dark board with the stage strip, run controls, live log,
            reject lane and repair routes, exactly as built. Opens in a
            new tab.</div>
        </div>
        <a class="pa-entry-btn" href="/sft-pipeline-map" target="_blank"
           rel="noopener">Open pipeline map
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
               stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg></a>
      </div>

      <div class="pa-entry">
        <div>
          <div class="pa-entry-t">Code explorer</div>
          <div class="pa-entry-s">Read the real Python implementation — pipeline.py, rules.py,
            lm_score.py and the rest — with symbol navigation.</div>
        </div>
        <a class="pa-entry-btn" href="#sft/code">Browse source
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
               stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg></a>
      </div>
    </div></div>`;
    return;
  }

  const t = s.totals || {};
  const kpis = [
    ['runs', fmtInt(t.runs), 'discovered output directories with report.json', 'var(--acc)'],
    ['rows scored', fmtInt(t.rows), `across ${fmtInt(t.runs)} separate input${t.runs === 1 ? '' : 's'}`, 'var(--blue)'],
    ['training-ready', fmtInt(t.high), 'routed to high/ by the quality gate', 'var(--acc)'],
    ['rejected', fmtInt(t.low), 'routed to low/ — inspect scores.jsonl', (t.low > 0) ? 'var(--amber)' : 'var(--ink-faint)'],
  ];

  let runRows;
  if (!(s.runs || []).length) {
    runRows = `<tr><td colspan="6" class="faint">No runs discovered yet. Run
      <code>python -m sftqc.pipeline data.jsonl --out qc_out</code> in the project
      to produce one.</td></tr>`;
  } else {
    runRows = s.runs.map(r => `<tr>
      <td class="mono">${esc(r.id)}</td>
      <td class="mono dim">${esc(r.input || '—')}</td>
      <td class="mono dim">${r.rows != null ? fmtInt(r.rows) : '—'}</td>
      <td class="mono">${r.high != null ? fmtInt(r.high) : '—'} <span class="dim">/</span>
        <span style="color:${r.low ? 'var(--amber)' : 'var(--ink-faint)'}">${r.low != null ? fmtInt(r.low) : '—'}</span></td>
      <td>${(r.layers || []).map(l => `<span class="stage-chip">${esc(l)}</span>`).join(' ')}</td>
      <td class="mono dim">${r.completedAtMs ? ago(r.completedAtMs) : '—'}</td>
    </tr>`).join('');
  }

  // Language mix + latest run detail for the newest run only.
  const latest = s.latest;
  const langRows = latest && (latest.languages || []).length
    ? latest.languages.map(l => {
        const pct = latest.rows ? (100 * l.rows / latest.rows) : 0;
        return `<div class="lang-row"><span class="lang-k">${esc(l.language)}</span>
          <div class="lang-bar"><i style="width:${pct.toFixed(1)}%"></i></div>
          <span class="lang-v">${fmtInt(l.rows)} · ${pct.toFixed(0)}%</span></div>`;
      }).join('')
    : '<div class="empty-note">No language breakdown recorded.</div>';

  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="kpis">${kpis.map(x =>
      `<div class="kpi" style="--kcol:${x[3]}"><div class="kpi-k">${x[0]}</div>
       <div class="kpi-v">${x[1]}</div><div class="kpi-s">${x[2]}</div></div>`).join('')}
    </div>

    <div class="pa-entry">
      <div>
        <div class="pa-entry-t">EKA SFT data pipeline</div>
        <div class="pa-entry-s">The six-stage board — Collect → Standardize →
          Clean corpus → Curate gold → Audit + handoff → Train + feedback —
          with each stage's rejected rows and where they are repaired.</div>
      </div>
      <a class="pa-entry-btn" href="#sft/board">Open pipeline board
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
             stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg></a>
    </div>

    <div class="pa-entry">
      <div>
        <div class="pa-entry-t">SFT pipeline map</div>
        <div class="pa-entry-s">The original EKA SFT data-pipeline board —
          the dark board with the stage strip, run controls, live log,
          reject lane and repair routes, exactly as built. Opens in a
          new tab.</div>
      </div>
      <a class="pa-entry-btn" href="/sft-pipeline-map" target="_blank"
         rel="noopener">Open pipeline map
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
             stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg></a>
    </div>

    <div class="pa-entry">
      <div>
        <div class="pa-entry-t">Code explorer</div>
        <div class="pa-entry-s">Read the real implementation behind every box —
          pipeline.py, rules.py, lang.py, lm_score.py, dedup.py, judge.py,
          selftest.py — with symbol navigation.</div>
      </div>
      <a class="pa-entry-btn" href="#sft/code">Browse source
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
             stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg></a>
    </div>

    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Run history</div>
      <div class="panel-sub">each row is one sftqc execution, read from its report.json</div>
    </div></div>
      <div class="table-scroll"><table>
        <thead><tr><th>run</th><th>input</th><th>rows</th><th>high / low</th><th>layers executed</th><th>completed</th></tr></thead>
        <tbody>${runRows}</tbody></table></div></div>

    <div class="detail-grid">
      <div class="panel"><div class="panel-h"><div>
        <div class="panel-t">Language mix — ${esc(latest ? latest.id : 'latest run')}</div>
        <div class="panel-sub">rows per detected language</div>
      </div></div><div style="padding:14px 18px">${langRows}</div></div>

      <div class="panel"><div class="panel-h"><div>
        <div class="panel-t">Latest run</div>
        <div class="panel-sub">configuration recorded by the pipeline</div>
      </div></div><div style="padding:14px 18px">${
        latest ? `
        <div class="dm"><div class="dm-k">scoring model</div><div class="dm-v">${esc(latest.model || 'none (--no-lm)')}</div></div>
        <div class="dm"><div class="dm-k">language filter</div><div class="dm-v">${esc(latest.langFilter || 'none')}</div></div>
        <div class="dm"><div class="dm-k">min score</div><div class="dm-v">${latest.settings && latest.settings.minScore != null ? latest.settings.minScore : '—'}</div></div>
        <div class="dm"><div class="dm-k">warn routing</div><div class="dm-v">${esc((latest.settings && latest.settings.warnTo) || '—')}</div></div>
        <div class="dm"><div class="dm-k">outputs</div><div class="dm-v">${(latest.outputs || []).map(esc).join('<br>') || '—'}</div></div>
        <div class="dm"><div class="dm-k">duration</div><div class="dm-v">${latest.seconds != null ? latest.seconds.toFixed(1) + 's' : '—'}</div></div>
        <div class="dm"><div class="dm-k">output path</div><div class="dm-v">${esc(latest.path)}</div></div>` :
        '<div class="empty-note">No runs yet.</div>'
      }</div></div>
    </div>

    ${(latest && ((latest.errors || []).length || (latest.warnings || []).length)) ? `
    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Flags in the latest run — ${esc(latest.id)}</div>
      <div class="panel-sub">every flag code the report records, with its row count</div>
    </div></div>
      <div style="padding:14px 18px">
        ${[['error', latest.errors || [], 'var(--red)'], ['warning', latest.warnings || [], 'var(--amber)']]
          .filter(([, list]) => list.length)
          .map(([sev, list, col]) => `
            <div class="sx-flag-group">
              <div class="sx-flag-sev" style="color:${col}">${sev}</div>
              ${list.map(f => `<div class="reason-row"><span class="reason-k" style="color:${col}">${esc(f.code)}</span>
                <span class="reason-v">${fmtInt(f.rows)} rows</span></div>`).join('')}
            </div>`).join('') || '<div class="empty-note">No flags raised.</div>'}
      </div></div>` : ''}

    ${(s.selftests || []).length ? `
    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Selftest — measured check quality</div>
      <div class="panel-sub">planted problems caught by the expected check · from selftest_results.json</div>
    </div></div>
      <div style="padding:14px 18px;display:grid;gap:16px">
        ${s.selftests.map(st => `
          <div>
            <div class="dm"><div class="dm-k">${esc(st.id)}</div>
              <div class="dm-v">${st.cleanRowsInLow != null
                ? `${st.cleanRowsInLow}/${st.cleanTotal ?? '—'} clean rows wrongly dropped`
                : 'clean-row count not recorded'} · ${st.sweepPoints} threshold settings swept</div></div>
            ${(st.kinds || []).map(k => {
              const rate = k.caught != null && k.total ? Math.round(100 * k.caught / k.total) : null;
              const col = rate == null ? 'var(--ink-faint)' : rate >= 90 ? 'var(--acc)' : rate >= 70 ? 'var(--amber)' : 'var(--red)';
              return `<div class="lang-row"><span class="lang-k" title="${esc(k.kind)}">${esc(k.kind.replace(/_/g, ' '))}</span>
                <div class="lang-bar"><i style="width:${rate ?? 0}%;background:${col}"></i></div>
                <span class="lang-v">${rate != null ? rate + '% caught' : '—'} · ${k.caught ?? '—'}/${k.total}</span></div>`;
            }).join('')}
          </div>`).join('')}
      </div></div>` : ''}

    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">How runs are executed</div>
      <div class="panel-sub">this workspace is read-only; the commands below are the real runners</div>
    </div></div>
      <div style="padding:14px 18px;font-family:var(--mono);font-size:11.5px;line-height:2;color:var(--ink-dim)">
        <div><span class="dim"># full pipeline over an OpenAI-format JSONL file</span></div>
        <div>python -m sftqc.pipeline data.jsonl --out qc_out [--model Qwen/Qwen2.5-1.5B-Instruct]</div>
        <div style="height:8px"></div>
        <div><span class="dim"># measure check recall on known-bad injections</span></div>
        <div>python -m sftqc.selftest clean.jsonl --out selftest_out</div>
        <div style="height:8px"></div>
        <div><span class="dim"># project location on this machine</span></div>
        <div>${esc(s.projectPath || '')}</div>
      </div></div>
  </div></div>`;
  $('#refreshNote').textContent = 'updated ' +
    new Date().toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

/* ============================================================
   SFT DATATROVE WORKFLOW CANVAS (#sft/workflow)
   ------------------------------------------------------------
   Interactive map of the documented sftqc quality-gate pipeline.
   Sources (nothing invented):

     · ~/soket/plan.md            — the SFT data-quality plan: the 18
                                    quality dimensions, layer table,
                                    decision rules and next steps
     · ~/soket/sftqc/pipeline.py  — the real execution order:
                                    rules → LM → embeddings → judge,
                                    then scoring and high/low routing
     · ~/soket/sftqc/{rules,lm_score,dedup,judge,lang}.py

   The engine (pan/zoom/fit/reset/details/fullscreen) is shared with
   the pretraining architecture canvas; only the data differs.
   ============================================================ */

/** SFT world geometry. */
const SW_World = { w: 2560, h: 1240 };

/** Six processing groups, left to right, in documented execution order. */
const SW_GROUPS = [
  { id: 'input',   title: 'Input',           sub: 'OpenAI chat JSONL',
    x: 40,   y: 150, w: 330, h: 760, hue: 'blue',
    blurb: 'What enters the gate. Rows are OpenAI chat-format JSONL in English, Hindi and Hinglish; English comes first. Every row is checked against all 18 quality dimensions in the plan.' },
  { id: 'rules',   title: 'Layer 1 · Rules', sub: 'deterministic checks',
    x: 420,  y: 150, w: 400, h: 760, hue: 'cyan',
    blurb: 'Checks with a definite yes/no answer and no model involved. This is the cheapest layer, so it runs first and drops everything it can prove is broken before any GPU work happens.' },
  { id: 'lm',      title: 'Layer 2 · LM audit', sub: 'per-token loss',
    x: 870,  y: 150, w: 380, h: 760, hue: 'violet',
    blurb: 'A causal language model scores every token the trainer would train on. Junk tokens stand out as high-loss surprises; whole rows that read unnaturally stand out against their language peers.' },
  { id: 'embed',   title: 'Layer 3a · Embeddings', sub: 'near-dup & conflicts',
    x: 1310, y: 150, w: 380, h: 760, hue: 'green',
    blurb: 'Multilingual sentence embeddings catch what hashing misses: paraphrased copies across languages, and the same question answered differently.' },
  { id: 'judge',   title: 'Layer 3b · LLM judge', sub: 'optional rubric',
    x: 1740, y: 150, w: 380, h: 760, hue: 'amber',
    blurb: 'An optional LLM grades each row on a six-criterion rubric. Built but not yet run against a validated judge model — the plan gates turning it on until it agrees with hand labels over 85% of the time.' },
  { id: 'route',   title: 'Scoring & routing', sub: 'high/ or low/',
    x: 2170, y: 150, w: 350, h: 760, hue: 'pink',
    blurb: 'Flags become a 0-100 score, and the score plus severities decide where each row lands. Nothing is deleted: low/ rows are kept for inspection with the reasons attached.' },
];

/** Nodes. `status`: 'built' | 'planned' | 'optional'. */
const SW_NODE = (id, title, sub, status, x, y, w, h) =>
  ({ id, title, sub, status, x, y, w, h });

const SW_NODES = [
  // --- Input ---
  SW_NODE('chat', 'Chat rows (JSONL)', 'messages[] · tools[] · OpenAI format', 'built', 66, 230, 278, 96),
  SW_NODE('langs_in', 'en · hi · hinglish', 'English first, then Hindi and Hinglish', 'built', 66, 348, 278, 96),

  // --- Layer 1: rules ---
  SW_NODE('format', 'Format & structure', 'bad JSON · wrong roles · empty turns · last turn must be assistant', 'built', 446, 230, 348, 96),
  SW_NODE('toolcheck', 'Tool-call consistency', 'call without result · wrong name · invalid arguments', 'built', 446, 348, 348, 96),
  SW_NODE('encoding', 'Encoding damage', 'U+FFFD · mojibake â€™ · control chars · leaked <|im_end|>', 'built', 446, 466, 348, 96),
  SW_NODE('incomplete', 'Incompleteness', 'answer cut off mid-sentence', 'built', 446, 584, 348, 88),
  SW_NODE('exactdup', 'Exact duplicates', 'sha256 of messages · keep first copy', 'built', 446, 690, 348, 88),

  // --- Language ID (runs inside layer 1) ---
  SW_NODE('langid', 'Language ID', 'script share + function words → en/hi/hinglish/mixed/other', 'built', 446, 798, 348, 88),

  // --- Layer 2: LM ---
  SW_NODE('template', 'Template & masking audit', 'loss must land on assistant turns only', 'built', 896, 230, 328, 96),
  SW_NODE('tokaudit', 'Tokenizer audit', 'partial-byte tokens · chars per token', 'built', 896, 348, 328, 96),
  SW_NODE('junk', 'Junk-token detection', 'stray "?" · symbols the model did not expect', 'built', 896, 466, 328, 96),
  SW_NODE('unnatural', 'Unnatural text', 'robust z-score vs same-language rows', 'built', 896, 584, 328, 96),

  // --- Layer 3a: embeddings ---
  SW_NODE('neardup', 'Near-duplicates', 'paraphrase + cross-language copies · sim ≥ 0.95', 'built', 1336, 230, 328, 108),
  SW_NODE('conflict', 'Conflicting answers', 'same prompt, different answer · review', 'built', 1336, 364, 328, 108),

  // --- Layer 3b: judge ---
  SW_NODE('rubric', 'Six-criterion rubric', 'grounded · correct · helpful · language_match · clean_text · tool_use', 'built', 1766, 230, 328, 118),
  SW_NODE('policy', 'Knowledge-cutoff policy', 'search after Jan 2026 · answer earlier facts directly', 'built', 1766, 374, 328, 108),

  // --- Routing ---
  SW_NODE('score', 'Composite score', '0-100 · weights lm 0.35 / judge 0.65', 'built', 2196, 230, 298, 96),
  SW_NODE('decision', 'Decision', 'error → low · warn → low (default) · score < 60 → low', 'built', 2196, 348, 298, 108),
];

/** Outputs. */
const SW_OUTPUTS = [
  { id: 'out_high', label: 'high/<name>.jsonl', sub: 'training-ready rows — train on this', x: 2196, y: 500, w: 298, h: 84 },
  { id: 'out_low',  label: 'low/<name>.jsonl',  sub: 'dropped rows, kept with reasons',      x: 2196, y: 604, w: 298, h: 84 },
  { id: 'out_scores', label: 'scores.jsonl',   sub: 'per-row flags, score and reasons',      x: 2196, y: 708, w: 298, h: 84 },
  { id: 'out_report', label: 'report.json / .md', sub: 'run summary, calibration, counts',   x: 2196, y: 812, w: 298, h: 84 },
];

/** The test harness runs beside the pipeline, not inside it. */
const SW_SIDECAR = {
  x: 40, y: 950, w: 1020, h: 130,
  title: 'selftest.py — measures the checks themselves',
  sub: 'Plants 140 known problems (junk tokens, corruption, wrong language, cut-offs) and reports the catch rate and how many clean rows were wrongly dropped. Latest measured result: junk/corruption/language 100%, cut-offs 95%, clean rows wrongly dropped 0/20.',
};

/** Planned extensions (plan §4 Phase A) — documented, not built. */
const SW_PLANNED = {
  x: 1090, y: 950, w: 1430, h: 130,
  title: 'Planned dimensions (plan §4 Phase A) — not built yet',
  sub: 'Refusal & boilerplate detection · length checks (too short / too long, relative to the question, per language) · personal-data detector (emails, phone numbers, Aadhaar/PAN) · dataset balance via embedding clusters. Each will be added to selftest.py so its catch rate is measured.',
};

/** Connectors in documented execution order. */
const SW_EDGES = [
  ['chat','format'], ['chat','toolcheck'], ['chat','encoding'],
  ['langs_in','langid'],
  ['format','template'], ['toolcheck','template'], ['encoding','template'],
  ['incomplete','template'], ['exactdup','template'], ['langid','template'],
  ['langid','junk'],
  ['template','neardup'], ['tokaudit','neardup'], ['junk','neardup'], ['unnatural','neardup'],
  ['neardup','rubric'], ['conflict','rubric'],
  ['rubric','score'], ['policy','score'],
  ['score','decision'],
  ['decision','out_high'], ['decision','out_low'],
  ['score','out_scores'], ['decision','out_report'],
];

/** Details panel content per node. */
const SW_DETAILS = {
  chat: { group: 'Input', body: [
    ['Format', 'One JSON object per line, OpenAI chat format: messages[] with role/content pairs, optional tools[]. This is exactly what the trainer consumes, so the gate never has to convert anything.'],
    ['Languages', 'English, Hindi and Hinglish. English comes first in the plan; the other two follow in Phase E after their own selftest and spot-checks.'],
    ['Scale note', 'The dashboard reads report.json only — the JSONL contents are never opened, so a run over millions of rows costs one small JSON parse.'],
  ]},
  langs_in: { group: 'Input', body: [
    ['Why three', 'The dataset mixes English, Hindi and Hinglish (Hindi written in Latin script). Wrong-language answers are one of the 18 quality dimensions.'],
    ['Hinglish is hard', 'Generic n-gram detectors confused short, name-heavy English questions with Hinglish, which is why sftqc uses function-word matching instead.'],
    ['Tokenization risk', '49% of Hindi tokens are partial characters — a likely source of � or ? in generated text, and the reason the tokenizer audit exists.'],
  ]},
  format: { group: 'Layer 1 · Rules', body: [
    ['Checks', 'Invalid JSON · wrong roles · empty turns · a final turn that is not from the assistant. Any of these means the row cannot be trained on at all.'],
    ['Dimension', 'Plan dimension 1 (broken format) — status ✅ built.'],
    ['Cost', 'Pure Python, no model. Runs on every row before anything expensive.'],
  ]},
  toolcheck: { group: 'Layer 1 · Rules', body: [
    ['Checks', 'A tool call with no matching result · a result with no call · an unknown tool name · tool-call arguments that do not parse.'],
    ['Dimension', 'Plan dimension 2 (broken tool use) — status ✅ built.'],
    ['Why it matters', 'Rows that pair calls with nothing teach the model to hallucinate tool output.'],
  ]},
  encoding: { group: 'Layer 1 · Rules', body: [
    ['Checks', 'U+FFFD replacement characters · mojibake sequences such as â€™ · raw control characters · leaked special tokens like <|im_end|>.'],
    ['Fixable subset', 'zero_width and not_nfc are repairable; mojibake is repaired when the ftfy package is available. With --fix the row is mended and kept instead of dropped.'],
    ['Dimension', 'Plan dimension 3 (corrupted text) — status ✅ built.'],
  ]},
  incomplete: { group: 'Layer 1 · Rules', body: [
    ['Check', 'The final assistant turn ends mid-sentence — a truncated generation that would teach the model to stop arbitrarily.'],
    ['Dimension', 'Plan dimension 5 (incomplete) — status ✅ built. Measured catch rate on planted problems: 95%.'],
  ]},
  exactdup: { group: 'Layer 1 · Rules', body: [
    ['How', 'sha256 over the sorted message payload; the first copy is kept and later copies are flagged exact_duplicate.'],
    ['Dimension', 'Plan dimension 7 (duplicates, exact half) — status ✅ built. Paraphrased copies need embeddings and are handled in layer 3a.'],
  ]},
  langid: { group: 'Layer 1 · Rules', body: [
    ['Method', 'Devanagari share decides Hindi; a fixed list of function words decides English vs Hinglish, because those words form a small closed class in each language. Labels: en, hi, hinglish, mixed, other.'],
    ['Why not a library', 'lingua, fasttext and small LMs were tried on this data and confused short English questions with Hinglish. Function words were more reliable.'],
    ['Filter', 'With --lang en, rows in other languages are dropped before the expensive layers. The count appears as filtered_by_lang in the report.'],
    ['Dimension', 'Plan dimension 6 (wrong language) — status ✅ built. Catch rate 100% on planted problems.'],
  ]},
  template: { group: 'Layer 2 · LM audit', body: [
    ['What it verifies', 'The chat template renders the row correctly, and loss lands only on assistant turns, each ending with the end-of-turn token. report.md shows the exact spans that would be trained.'],
    ['Why it matters', 'If the template is wrong the model learns escaped JSON or trains on user turns — silent quality damage no downstream metric catches.'],
    ['Sample', 'The audit runs on one representative row (preferring a row with tool_calls) and its findings go into the report.'],
  ]},
  tokaudit: { group: 'Layer 2 · LM audit', body: [
    ['Measures', 'Partial-byte token share and characters-per-token, per language.'],
    ['Signals', 'A high partial-byte share means the tokenizer is splitting characters — the suspected cause of � and ? appearing in Hindi generations.'],
  ]},
  junk: { group: 'Layer 2 · LM audit', body: [
    ['Rule', 'A symbolic token counts as junk when its negative log-likelihood is high, the model put almost no probability on any symbol there, and it is clearly more surprising than the row\'s typical token.'],
    ['Calibration', 'Thresholds (nll ≥ 10, p_sym < 1.0, margin) are recorded in report.json under lm_calibration, alongside the median symbolic-token loss actually observed.'],
    ['Dimension', 'Plan dimension 4 (junk tokens, including the stray "?") — status ✅ built. Catch rate 100% on planted problems.'],
  ]},
  unnatural: { group: 'Layer 2 · LM audit', body: [
    ['How', 'Each row\'s mean content loss is compared with other rows in the same language using a robust z-score (median/MAD).'],
    ['Both tails flag', 'High z = reads unnaturally (garbled or machine-translated). Low z = unusually predictable, which suggests boilerplate or memorised text.'],
    ['Group size', 'Needs at least 8 rows in a language before the z-score is meaningful; smaller groups are noted in the report instead.'],
    ['Dimension', 'Plan dimension 9 (unnatural text) — status ✅ built.'],
  ]},
  neardup: { group: 'Layer 3a · Embeddings', body: [
    ['Model', 'sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2, multilingual so cross-language copies are caught.'],
    ['Rule', 'Whole-conversation similarity ≥ 0.95 against an earlier row → near_duplicate; the first copy is kept.'],
    ['Scale', 'Similarity is computed in chunks of 2048, fine to ~100k rows; beyond that the module expects an ANN index (faiss).'],
    ['Dimension', 'Plan dimension 7 (duplicates, paraphrase half) — status ✅ built.'],
  ]},
  conflict: { group: 'Layer 3a · Embeddings', body: [
    ['Rule', 'Prompt similarity ≥ 0.92 to an earlier row while answer similarity < 0.80 → conflicting_answer.'],
    ['Action', 'Review, not drop: the evidence is uncertain. By default such rows still go to low/, but --warn-to high keeps them.'],
    ['Dimension', 'Plan dimension 8 (contradictions) — status ✅ built.'],
  ]},
  rubric: { group: 'Layer 3b · LLM judge', body: [
    ['Criteria (each 1-5)', 'grounded — claims supported by tool results · correct — accurate and consistent with the cutoff policy · helpful — answers what was asked, no padding · language_match — same language/script/register as the user · clean_text — no stray characters or broken markup · tool_use — searched when required, not otherwise.'],
    ['Output', 'Judge scores feed the composite (weight 0.65); criteria below threshold become drop flags for dimensions 10-12.'],
    ['Status', 'Built but not yet run against a validated judge. Plan Phase B gates it: 200 hand-labelled rows, target >85% agreement, before it is trusted.'],
  ]},
  policy: { group: 'Layer 3b · LLM judge', body: [
    ['The policy text', 'Knowledge cutoff January 2026. The assistant should use web_search for events from January 2026 onward, scheduled future events, or anything "current"/"latest"; answer earlier facts directly; never guess post-cutoff facts.'],
    ['Why it is explicit', 'Without a stated policy the judge cannot decide whether searching was correct, so tool_use and correct both become noise.'],
  ]},
  score: { group: 'Scoring & routing', body: [
    ['Formula', 'Components are normalised to 0-1 and weighted: lm 0.35, judge 0.65. A row with no model components scores 100 if usable, 0 if not.'],
    ['Threshold', '--min-score, default 60. A row below it is dropped even when no single check failed.'],
  ]},
  decision: { group: 'Scoring & routing', body: [
    ['Routing rules', 'Any error → low/. Any warning → low by default, or high with --warn-to high. Score below min-score → low. Rows outside the --lang filter are excluded entirely (filtered_by_lang).'],
    ['Never deleted', 'low/ keeps the rejected rows so a person can inspect them; scores.jsonl carries every flag and the reason.'],
  ]},
  out_high: { group: 'Output', body: [
    ['Contents', 'Rows that passed the gate, in the same format as the input — the trainer consumes them unchanged.'],
    ['Downstream', 'Plan Phase D trains two runs (high/ only vs all data) on 500 fixed prompts and keeps the filter only if the high/ run is at least as good.'],
  ]},
  out_low: { group: 'Output', body: [
    ['Contents', 'Rejected rows with their flags preserved.'],
    ['Use', 'Spot-check 100 random low/ rows to measure how many good rows are being lost; if more than 30% of the dataset is dropped, the plan says to re-check the thresholds before accepting.'],
  ]},
  out_scores: { group: 'Output', body: [
    ['Contents', 'One record per input row: every flag with severity and reason, the component scores and the final 0-100 score.'],
  ]},
  out_report: { group: 'Output', body: [
    ['Contents', 'report.json (machine-readable: settings, counts, language mix, LM calibration) and report.md (human summary). This is the only file the dashboard reads.'],
  ]},
};

/* ============================================================
   PRETRAINING ARCHITECTURE CANVAS (#pretrain/architecture)
   ------------------------------------------------------------
   An interactive, zoomable/pannable map of the documented pretraining
   data architecture. Every group, node and connector below is taken
   from the project's own documentation:

     · ~/soket/codepipe/README.md      — built stages 0-3, their commands
                                         and output layout
     · ~/soket/code_pretrain_plan.md   — planned stages 4-6, quality
                                         classifier detail, final split

   Nothing is invented: nodes marked "planned" are the README's "Next
   stages (not built yet)" and the plan's Sections 2-4. The visual
   grouping mirrors the reference architecture diagram supplied with
   this task (collection → extraction → filtering → preprocessing →
   classification → tokenization).
   ============================================================ */

/** Build a self-contained view controller for one diagram spec. */
function paMakeEngine(spec) {
  const st = {
    mounted: false, epoch: 0, cleanup: [],
    scale: 1, tx: 0, ty: 0,
    selected: null, dragging: false, dragStart: null, moved: false,
  };

  const byId = (id) =>
    spec.nodes.find(n => n.id === id)
    || spec.outputs.find(n => n.id === id)
    || spec.groups.find(n => n.id === id) || null;

  function bounds() {
    let x0=1e9,y0=1e9,x1=-1e9,y1=-1e9;
    const grow=(n)=>{x0=Math.min(x0,n.x);y0=Math.min(y0,n.y);
                     x1=Math.max(x1,n.x+n.w);y1=Math.max(y1,n.y+n.h);};
    spec.groups.forEach(grow); spec.nodes.forEach(grow); spec.outputs.forEach(grow);
    for (const lane of spec.lanes||[]) grow(lane);
    return {x:x0,y:y0,w:x1-x0,h:y1-y0};
  }

  function apply(scale,tx,ty,animate) {
    const world = $('#paWorld'); if (!world) return;
    st.scale=scale; st.tx=tx; st.ty=ty;
    world.style.transition = animate ? 'transform .34s cubic-bezier(.22,.7,.3,1)' : 'none';
    world.style.transform = `translate(${tx}px,${ty}px) scale(${scale})`;
    const z=$('#paZoomPct'); if (z) z.textContent = Math.round(scale*100)+'%';
  }

  function fit(animate) {
    const vp=$('#paViewport'); if(!vp) return;
    const b=bounds(), pad=46;
    const sc=Math.min((vp.clientWidth-pad*2)/b.w,(vp.clientHeight-pad*2)/b.h,1.6);
    apply(sc,(vp.clientWidth-b.w*sc)/2-b.x*sc,(vp.clientHeight-b.h*sc)/2-b.y*sc,animate);
  }
  function reset(){ apply(1,40,20,true); }

  function zoomAt(cx,cy,factor) {
    const next=Math.min(2.4,Math.max(0.28,st.scale*factor));
    if(next===st.scale) return;
    const k=next/st.scale;
    apply(next, cx-(cx-st.tx)*k, cy-(cy-st.ty)*k, false);
  }

  function highlightEdges(id) {
    document.querySelectorAll('.pa-edge').forEach(e=>{
      const on = id && (e.dataset.from===id || e.dataset.to===id);
      e.classList.toggle('pa-edge-on', !!on);
    });
  }

  function showDetails(id) {
    st.selected=id;
    const node=byId(id), det=spec.details[id];
    const panel=$('#paDetailBody'), title=$('#paDetailTitle'),
          grp=$('#paDetailGroup'), badge=$('#paDetailBadge');
    if(!panel) return;
    document.querySelectorAll('.pa-node').forEach(n=>
      n.classList.toggle('pa-node-sel', n.dataset.id===id));
    if(!node||!det){
      title.textContent=spec.emptyTitle;
      grp.textContent=spec.emptySub;
      badge.style.display='none';
      panel.innerHTML=`<div class="pa-detail-empty">${spec.emptyBody}</div>`;
      highlightEdges(null); return;
    }
    title.textContent=node.title;
    grp.textContent=det.group;
    const planned = node.status==='planned';
    badge.style.display='';
    badge.textContent = node.status==='optional' ? 'optional layer'
                      : planned ? 'planned · not built yet' : 'built in sftqc';
    badge.className='pa-badge '+(planned?'pa-badge-planned':'pa-badge-built');
    panel.innerHTML=det.body.map(([k,v])=>`
      <div class="pa-detail-row">
        <div class="pa-detail-k">${esc(k)}</div>
        <div class="pa-detail-v">${esc(v)}</div>
      </div>`).join('');
    highlightEdges(id);
  }

  /** SVG for one group frame. */
  const groupSvg = (g)=>`
    <g class="pa-group" data-id="${g.id}">
      <rect class="pa-group-box pa-hue-${g.hue}" x="${g.x}" y="${g.y}"
            width="${g.w}" height="${g.h}" rx="18"/>
      <text class="pa-group-title" x="${g.x+24}" y="${g.y+42}">${esc(g.title)}</text>
      <text class="pa-group-sub"   x="${g.x+24}" y="${g.y+66}">${esc(g.sub)}</text>
    </g>`;

  /** SVG for one selectable node. */
  const nodeSvg = (n)=>{
    const cls = n.status==='planned' ? 'pa-node-planned' :
                n.status==='optional' ? 'pa-node-optional' : '';
    const outCls = n.isOut ? 'pa-node-out' : '';
    return `<g class="pa-node ${cls} ${outCls}" data-id="${n.id}" tabindex="0" role="button"
              aria-label="${esc(n.title)} — ${esc(n.sub)}">
      <rect class="pa-node-box ${n.isOut?'pa-node-box-out':''}" x="${n.x}" y="${n.y}"
            width="${n.w}" height="${n.h}" rx="${n.isOut?10:12}"/>
      <foreignObject x="${n.x+14}" y="${n.y+11}" width="${n.w-28}" height="${n.h-22}">
        <div xmlns="http://www.w3.org/1999/xhtml" class="pa-node-inner">
          <div class="pa-node-title">${esc(n.title)}</div>
          <div class="pa-node-sub">${esc(n.sub)}</div>
        </div>
      </foreignObject>
    </g>`;
  };

  const laneSvg = (l)=>`
    <g class="pa-lane" tabindex="0" role="button" aria-label="${esc(l.title)}">
      <rect class="pa-lane-box ${l.planned?'pa-lane-planned':''}" x="${l.x}" y="${l.y}"
            width="${l.w}" height="${l.h}" rx="14"/>
      <foreignObject x="${l.x+22}" y="${l.y+15}" width="${l.w-44}" height="${l.h-30}">
        <div xmlns="http://www.w3.org/1999/xhtml" class="pa-lane-inner">
          <div class="pa-lane-title">${esc(l.title)}</div>
          <div class="pa-lane-sub">${esc(l.sub)}</div>
        </div>
      </foreignObject>
    </g>`;

  function edgesSvg() {
    return spec.edges.map(([a,b])=>{
      const A=byId(a),B=byId(b); if(!A||!B) return '';
      const ax=A.x+A.w, ay=A.y+A.h/2, bx=B.x, by=B.y+B.h/2;
      const mx=(ax+bx)/2;
      const d=`M ${ax} ${ay} C ${mx} ${ay}, ${mx} ${by}, ${bx-7} ${by}`;
      return `<path class="pa-edge" data-from="${a}" data-to="${b}" d="${d}"/>`;
    }).join('');
  }

  function worldSvg() {
    return `
    <svg id="paSvg" viewBox="0 0 ${spec.world.w} ${spec.world.h}"
         width="${spec.world.w}" height="${spec.world.h}"
         role="img" aria-label="${esc(spec.ariaLabel)}">
      <defs>
        <marker id="paArrow" viewBox="0 0 10 10" refX="9" refY="5"
                markerWidth="7" markerHeight="7" orient="auto-start-reverse">
          <path d="M 0 0 L 10 5 L 0 10 z" class="pa-arrow"/>
        </marker>
      </defs>
      ${spec.groups.map(groupSvg).join('')}
      ${edgesSvg()}
      ${(spec.lanes||[]).map(laneSvg).join('')}
      ${spec.outputs.map(nodeSvg).join('')}
      ${spec.nodes.map(nodeSvg).join('')}
    </svg>`;
  }

  return { st, byId, bounds, apply, fit, reset, zoomAt,
           highlightEdges, showDetails, worldSvg };
}

/* ---- pretraining canvas state (engine-owned) ----------------------- */
/** Shell-side listener registry for the iframe-mounted architecture view
 *  (the component inside the iframe cleans up after itself when removed). */
let PA_SHELL_CLEANUP = [];

/** Mount the architecture view: the prebuilt pipeline-architecture page
 *  (assets/pipeline_architecture.html, served verbatim at
 *  /pipeline-architecture) embedded in an iframe. The component owns all
 *  of its styles, scripts, interactions, animations and exports; this
 *  shell adds only routing, a toolbar, fullscreen and lifecycle cleanup.
 *  The iframe is recreated on every mount so the component's state never
 *  leaks between visits, and unmounting it drops its listeners at once. */
async function renderPretrainArchitecture() {
  $('#pageTitle').textContent = 'Pretraining Architecture';
  $('#pageSub').textContent = 'live map of the EKA pretraining data pipeline';

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner pa-wrap">
    <div class="pa-toolbar">
      <div class="pa-toolbar-l">
        <button type="button" class="pa-btn" id="paBackBtn" aria-label="Back to stage view">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
               stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M19 12H5"/><path d="m12 19-7-7 7-7"/></svg>
          Stages</button>
        <span class="pa-sep"></span>
        <a class="pa-open-standalone" href="/pipeline-architecture" target="_blank"
           rel="noopener">Open standalone ↗</a>
      </div>
      <div class="pa-toolbar-r">
        <button type="button" class="pa-btn" id="paFsBtn" aria-pressed="false"
                aria-label="Toggle fullscreen view">Fullscreen</button>
      </div>
    </div>
    <div class="pa-frame-box" id="paFrameBox">
      <iframe id="paFrame" src="/pipeline-architecture"
              title="EKA pretraining data pipeline architecture"
              class="pa-frame"></iframe>
    </div>
    <div class="pa-note-line">Embedded verbatim from
      <code>assets/pipeline_architecture.html</code> — labels, flows,
      animations and exports are the original implementation's.</div>
  </div></div>`;

  // ---- lifecycle: only the shell's own listeners need cleanup ----
  const off = (el, ev, fn) => {
    el.addEventListener(ev, fn);
    PA_SHELL_CLEANUP.push(() => el.removeEventListener(ev, fn));
  };

  off($('#paBackBtn'), 'click', () => { location.hash = '#pretrain'; });
  off($('#paFsBtn'), 'click', paToggleFullscreen);
  off(document, 'fullscreenchange', paFsReflect);
  off(document, 'webkitfullscreenchange', paFsReflect);
}

/** Fullscreen for either pipeline canvas panel (FTFS conventions). */
async function paToggleFullscreen() {
  const panel = $('#paPanel') || $('.pa-wrap');
  if (!panel) return false;
  if (ftIsFullscreen()) {
    try { await document.exitFullscreen(); } catch { /* reflect reconciles */ }
    return !ftIsFullscreen();
  }
  try {
    if (panel.requestFullscreen) await panel.requestFullscreen();
    else if (panel.webkitRequestFullscreen) panel.webkitRequestFullscreen();
  } catch { /* browser refused */ }
  return ftIsFullscreen();
}

function paFsReflect(after) {
  // Called both directly (SFT passes a refit callback) and as an event
  // listener (the architecture view registers it verbatim), so ignore the
  // Event object when it lands in `after`.
  const refit = typeof after === 'function' ? after : null;
  const b = $('#paFsBtn'); if (!b) return;
  const on = ftIsFullscreen();
  b.textContent = on ? 'Exit Fullscreen' : 'Fullscreen';
  b.setAttribute('aria-pressed', String(on));
  document.body.classList.toggle('pa-fs', on);
  // Refit after the layout change settles, using whichever engine is live.
  setTimeout(() => { if (refit) refit(); }, 90);
}

/** Mount the architecture report: the navigable engineering reference at
 *  assets/pretrain_report.html, served verbatim at /pretrain-report and
 *  embedded in an iframe. The report owns its own navigation, search,
 *  collapsible phases and anchors; this shell adds routing, a toolbar and
 *  lifecycle cleanup, exactly like the interactive map view. */
async function renderPretrainReport() {
  $('#pageTitle').textContent = 'Pretraining Report';
  $('#pageSub').textContent = 'the complete pipeline reference — every phase, stage and component';

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner pa-wrap">
    <div class="pa-toolbar">
      <div class="pa-toolbar-l">
        <button type="button" class="pa-btn" id="prBackBtn" aria-label="Back to stage view">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
               stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M19 12H5"/><path d="m12 19-7-7 7-7"/></svg>
          Stages</button>
        <span class="pa-sep"></span>
        <a class="pa-open-standalone" href="#pretrain/architecture">Interactive map ↗</a>
      </div>
      <div class="pa-toolbar-r">
        <a class="pa-open-standalone" href="/pretrain-report" target="_blank"
           rel="noopener">Open standalone ↗</a>
      </div>
    </div>
    <div class="pa-frame-box" id="paFrameBox">
      <iframe id="paFrame" src="/pretrain-report"
              title="Pretraining pipeline architecture report"
              class="pa-frame"></iframe>
    </div>
  </div></div>`;

  // ---- lifecycle: only the shell's own listener needs cleanup ----
  const off = (el, ev, fn) => {
    el.addEventListener(ev, fn);
    PA_SHELL_CLEANUP.push(() => el.removeEventListener(ev, fn));
  };
  off($('#prBackBtn'), 'click', () => { location.hash = '#pretrain'; });
}

/** Tear down listeners when leaving the pretraining canvas. */
function paTeardown() {
  // The architecture view is an embedded page: dropping its shell listeners
  // and letting #viewRoot re-render disposes of the component entirely.
  for (const off of PA_SHELL_CLEANUP.splice(0)) { try { off(); } catch { /* gone */ } }
  if (ftIsFullscreen()) ftExitFullscreenQuiet();
  document.body.classList.remove('pa-fs');
}

/* ---- SFT workflow view -------------------------------------------- */

/** The SFT diagram spec handed to the shared engine. */
function swSpec() {
  return {
    world: SW_World,
    groups: SW_GROUPS,
    nodes: SW_NODES,
    outputs: SW_OUTPUTS.map(o => ({ ...o, title: o.label, isOut: true })),
    lanes: [
      { ...SW_SIDECAR,   planned: false },
      { ...SW_PLANNED,   planned: true },
    ],
    edges: SW_EDGES,
    details: SW_DETAILS,
    ariaLabel: 'SFT data quality workflow diagram',
    emptyTitle: 'SFT DataTrove workflow',
    emptySub: 'select any component',
    emptyBody: `Click a component to read what it checks, how it decides, and
      where its numbers come from.<br><br>
      Groups are the pipeline's layers in execution order; nodes are individual
      checks; the outputs are what each run writes.`,
  };
}

let SW_ENGINE = null;

async function renderSftWorkflow() {
  const root = $('#viewRoot');
  $('#pageTitle').textContent = 'SFT Workflow';
  $('#pageSub').textContent = 'interactive map of the sftqc quality gate';

  const spec = swSpec();
  const eng  = paMakeEngine(spec);
  SW_ENGINE = eng;
  const PAE  = eng.st;

  root.innerHTML = `<div class="content"><div class="content-inner pa-wrap">
    <div class="pa-toolbar">
      <div class="pa-toolbar-l">
        <button type="button" class="pa-btn" id="paBackBtn" aria-label="Back to runs view">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
               stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M19 12H5"/><path d="m12 19-7-7 7-7"/></svg>
          Runs</button>
        <span class="pa-sep"></span>
        <button type="button" class="pa-btn pa-zoom-btn" id="paZoomOut" aria-label="Zoom out">−</button>
        <span class="pa-zoom-pct" id="paZoomPct">100%</span>
        <button type="button" class="pa-btn pa-zoom-btn" id="paZoomIn" aria-label="Zoom in">+</button>
        <span class="pa-sep"></span>
        <button type="button" class="pa-btn" id="paFitBtn" aria-label="Fit to screen">Fit</button>
        <button type="button" class="pa-btn" id="paResetBtn" aria-label="Reset view">Reset</button>
      </div>
      <div class="pa-toolbar-r">
        <span class="pa-demo-flag" title="This diagram documents the pipeline design. It shows no live metrics.">DEMO DATA · design reference</span>
        <button type="button" class="pa-btn" id="paFsBtn" aria-pressed="false"
                aria-label="Toggle fullscreen view">Fullscreen</button>
      </div>
    </div>

    <div class="pa-stage-row">
      <div class="pa-viewport" id="paViewport">
        <div class="pa-world" id="paWorld">${eng.worldSvg()}</div>
        <div class="pa-hint">drag to pan · scroll to zoom · click a component</div>
      </div>
      <aside class="pa-detail" id="paDetail" aria-live="polite">
        <div class="pa-detail-head">
          <div>
            <div class="pa-detail-group" id="paDetailGroup">select any component</div>
            <div class="pa-detail-title" id="paDetailTitle">SFT DataTrove workflow</div>
          </div>
          <span class="pa-badge pa-badge-built" id="paDetailBadge" style="display:none"></span>
        </div>
        <div class="pa-detail-body" id="paDetailBody"></div>
      </aside>
    </div>

    <div class="pa-legend">
      <span class="pa-lg"><i class="pa-lg-dot pa-lg-built"></i>built in sftqc</span>
      <span class="pa-lg"><i class="pa-lg-dot pa-lg-planned"></i>planned (plan §4)</span>
      <span class="pa-lg"><i class="pa-lg-arrow"></i>data flow</span>
      <span class="pa-note">Read-only documentation view · no jobs are executed.
        Source: plan.md + sftqc source.</span>
    </div>
  </div></div>`;

  // ---- interactions ----
  const off = (el, ev, fn, opt) => {
    el.addEventListener(ev, fn, opt);
    PAE.cleanup.push(() => el.removeEventListener(ev, fn, opt));
  };

  document.querySelectorAll('.pa-node').forEach(n => {
    off(n, 'click', (e) => { e.stopPropagation(); eng.showDetails(n.dataset.id); });
    off(n, 'keydown', (e) => {
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); eng.showDetails(n.dataset.id); }
    });
    off(n, 'focus', () => eng.highlightEdges(n.dataset.id));
    off(n, 'blur',  () => eng.highlightEdges(PAE.selected));
  });

  const vp = $('#paViewport');
  const startPan = (x,y)=>{ PAE.dragging=true; PAE.moved=false;
                            PAE.dragStart={x,y,tx:PAE.tx,ty:PAE.ty}; };
  const movePan = (x,y)=>{
    if(!PAE.dragging) return;
    const dx=x-PAE.dragStart.x, dy=y-PAE.dragStart.y;
    if(Math.abs(dx)+Math.abs(dy)>4) PAE.moved=true;
    eng.apply(PAE.scale, PAE.dragStart.tx+dx, PAE.dragStart.ty+dy, false);
  };

  off(vp,'pointerdown',(e)=>{
    if(e.target.closest('.pa-node')) return;
    vp.setPointerCapture(e.pointerId); startPan(e.clientX,e.clientY);
  });
  off(vp,'pointermove',(e)=>movePan(e.clientX,e.clientY));
  off(vp,'pointerup',()=>{PAE.dragging=false;});
  off(vp,'pointercancel',()=>{PAE.dragging=false;});
  off(vp,'click',(e)=>{
    if(PAE.moved) return;
    if(e.target.closest('.pa-node')) return;
    eng.showDetails(null);
  });
  off(vp,'wheel',(e)=>{
    e.preventDefault();
    const r=vp.getBoundingClientRect();
    eng.zoomAt(e.clientX-r.left, e.clientY-r.top, e.deltaY<0?1.13:1/1.13);
  },{passive:false});

  off($('#paZoomIn'),'click',()=>{const r=vp.getBoundingClientRect();
                                  eng.zoomAt(r.width/2,r.height/2,1.25);});
  off($('#paZoomOut'),'click',()=>{const r=vp.getBoundingClientRect();
                                   eng.zoomAt(r.width/2,r.height/2,1/1.25);});
  off($('#paFitBtn'),'click',()=>eng.fit(true));
  off($('#paResetBtn'),'click',()=>eng.reset());
  off($('#paBackBtn'),'click',()=>{location.hash='#sft';});
  off($('#paFsBtn'),'click',async()=>{await paToggleFullscreen();});
  off(document,'fullscreenchange',()=>paFsReflect(()=>eng.fit(true)));
  off(document,'webkitfullscreenchange',()=>paFsReflect(()=>eng.fit(true)));

  off(window,'keydown',(e)=>{
    if(e.target.closest('input,textarea,select')) return;
    if(ftEscapeKey(e)) return;
    if(e.key==='f'||e.key==='F'){e.preventDefault();paToggleFullscreen();}
    else if(e.key==='0'){e.preventDefault();eng.fit(true);}
    else if(e.key==='1'){e.preventDefault();eng.reset();}
  });

  const ro=new ResizeObserver(()=>{ if(PAE.scale<=0.001) eng.fit(false); });
  ro.observe(vp); PAE.cleanup.push(()=>ro.disconnect());

  eng.showDetails(null);
  requestAnimationFrame(()=>eng.fit(true));
  PAE.mounted = true;
}

/** Tear down the SFT canvas. */
function swTeardown() {
  if (SW_ENGINE) {
    SW_ENGINE.st.mounted = false; SW_ENGINE.st.epoch++;
    for (const off of SW_ENGINE.st.cleanup.splice(0)) { try { off(); } catch { /* gone */ } }
    SW_ENGINE = null;
  }
  if (ftIsFullscreen()) ftExitFullscreenQuiet();
  document.body.classList.remove('pa-fs');
}

/* ============================================================
   SFT PIPELINE ARCHITECTURE (#sft/architecture)
   ------------------------------------------------------------
   The prebuilt sft_architecture.html page (served verbatim at
   /sft-architecture) embedded in an iframe, exactly like the
   pretraining architecture view. The component owns its styles,
   scripts and interactions; this shell adds routing, a toolbar,
   fullscreen and lifecycle cleanup. The iframe is recreated on every
   mount so no state leaks between visits.
   ============================================================ */
async function renderSftArchitecture() {
  $('#pageTitle').textContent = 'SFT Pipeline Architecture';
  $('#pageSub').textContent = 'live map of the sftqc data-quality pipeline';

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner pa-wrap">
    <div class="pa-toolbar">
      <div class="pa-toolbar-l">
        <button type="button" class="pa-btn" id="saBackBtn" aria-label="Back to runs view">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
               stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M19 12H5"/><path d="m12 19-7-7 7-7"/></svg>
          Runs</button>
        <span class="pa-sep"></span>
        <a class="pa-open-standalone" href="/sft-architecture" target="_blank"
           rel="noopener">Open standalone ↗</a>
        <span class="pa-sep"></span>
        <a class="pa-open-standalone" href="#sft/code">Code explorer →</a>
      </div>
      <div class="pa-toolbar-r">
        <button type="button" class="pa-btn" id="saFsBtn" aria-pressed="false"
                aria-label="Toggle fullscreen view">Fullscreen</button>
      </div>
    </div>
    <div class="pa-frame-box" id="saFrameBox">
      <iframe id="saFrame" src="/sft-architecture"
              title="sftqc data quality pipeline architecture"
              class="pa-frame"></iframe>
    </div>
    <div class="pa-note-line">Embedded verbatim from
      <code>assets/sft_architecture.html</code> — every box maps to a real
      function in <code>~/soket/sftqc</code>; run counts come from
      <code>/api/pipelines/sft</code>. Nothing is executed.</div>
  </div></div>`;

  // ---- lifecycle: only the shell's own listeners need cleanup ----
  const off = (el, ev, fn) => {
    el.addEventListener(ev, fn);
    PA_SHELL_CLEANUP.push(() => el.removeEventListener(ev, fn));
  };
  off($('#saBackBtn'), 'click', () => { location.hash = '#sft'; });
  off($('#saFsBtn'), 'click', async () => {
    await paToggleFullscreen();
    const b = $('#saFsBtn'); if (b) {
      const on = ftIsFullscreen();
      b.textContent = on ? 'Exit Fullscreen' : 'Fullscreen';
      b.setAttribute('aria-pressed', String(on));
      document.body.classList.toggle('pa-fs', on);
    }
  });
  off(document, 'fullscreenchange', () => {
    const b = $('#saFsBtn'); if (!b) return;
    const on = ftIsFullscreen();
    b.textContent = on ? 'Exit Fullscreen' : 'Fullscreen';
    b.setAttribute('aria-pressed', String(on));
    document.body.classList.toggle('pa-fs', on);
  });

  // Deep link from the canvas: "open this function in the code explorer".
  // The iframe posts {type:'sft-open-code', symbol}; we route there.
  const onMsg = (ev) => {
    if (ev.source !== $('#saFrame')?.contentWindow) return;
    const d = ev.data;
    if (d && d.type === 'sft-open-code' && d.symbol) {
      location.hash = '#sft/code?f=' + encodeURIComponent(d.symbol);
    }
  };
  window.addEventListener('message', onMsg);
  PA_SHELL_CLEANUP.push(() => window.removeEventListener('message', onMsg));
}

/* ============================================================
   EKA SFT DATA-PIPELINE BOARD (#sft/board)
   ------------------------------------------------------------
   The prebuilt sft_board.html page (served verbatim at /sft-board)
   embedded in an iframe, exactly like the architecture views. The
   component owns its six-stage layout, run/reset animation, repair
   arrows and detail panel; this shell adds routing, a toolbar,
   fullscreen and lifecycle cleanup. The iframe is recreated on every
   mount so no state leaks between visits.
   ============================================================ */
async function renderSftBoard() {
  $('#pageTitle').textContent = 'EKA SFT Data Pipeline';
  $('#pageSub').textContent = 'six stages, their rejected rows and where each is repaired';

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner pa-wrap">
    <div class="pa-toolbar">
      <div class="pa-toolbar-l">
        <button type="button" class="pa-btn" id="sbBackBtn" aria-label="Back to runs view">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
               stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M19 12H5"/><path d="m12 19-7-7 7-7"/></svg>
          Runs</button>
        <span class="pa-sep"></span>
        <a class="pa-open-standalone" href="/sft-board" target="_blank"
           rel="noopener">Open standalone ↗</a>
        <span class="pa-sep"></span>
        <a class="pa-open-standalone" href="/sft-pipeline-map" target="_blank"
           rel="noopener">Pipeline map ↗</a>
      </div>
      <div class="pa-toolbar-r">
        <button type="button" class="pa-btn" id="sbFsBtn" aria-pressed="false"
                aria-label="Toggle fullscreen view">Fullscreen</button>
      </div>
    </div>
    <div class="pa-frame-box" id="sbFrameBox">
      <iframe id="sbFrame" src="/sft-board"
              title="EKA SFT data pipeline board"
              class="pa-frame"></iframe>
    </div>
    <div class="pa-note-line">Embedded verbatim from
      <code>assets/sft_board.html</code> — the six stages, run/reset
      animation, repair arrows and stage panel are the original
      implementation's.</div>
  </div></div>`;

  // ---- lifecycle: only the shell's own listeners need cleanup ----
  const off = (el, ev, fn) => {
    el.addEventListener(ev, fn);
    PA_SHELL_CLEANUP.push(() => el.removeEventListener(ev, fn));
  };
  off($('#sbBackBtn'), 'click', () => { location.hash = '#sft'; });
  off($('#sbFsBtn'), 'click', async () => {
    await paToggleFullscreen();
    const b = $('#sbFsBtn'); if (b) {
      const on = ftIsFullscreen();
      b.textContent = on ? 'Exit Fullscreen' : 'Fullscreen';
      b.setAttribute('aria-pressed', String(on));
      document.body.classList.toggle('pa-fs', on);
    }
  });
  off(document, 'fullscreenchange', () => {
    const b = $('#sbFsBtn'); if (!b) return;
    const on = ftIsFullscreen();
    b.textContent = on ? 'Exit Fullscreen' : 'Fullscreen';
    b.setAttribute('aria-pressed', String(on));
    document.body.classList.toggle('pa-fs', on);
  });
}

/* ============================================================
   SFT CODE EXPLORER (#sft/code)
   ------------------------------------------------------------
   Real source of the sftqc project, read through
   /api/pipelines/sft/code (index: files, symbols, docstrings) and
   /api/pipelines/sft/code/file?name=… (file text). Read-only; the
   project's own CLI remains the runner. Symbol chips scroll to the
   declaration; a light Python highlighter keeps it dependency-free.
   ============================================================ */
let SX = null; // {files:[{name,path,lines,symbols,doc}], sel, src}

function sxHighlight(py) {
  // Minimal, safe highlighting: escape first, then wrap tokens. Good
  // enough for reading structure; not a parser.
  const e = esc(py);
  return e
    .replace(/(&quot;&quot;&quot;[\s\S]*?&quot;&quot;&quot;|&#39;&#39;&#39;[\s\S]*?&#39;&#39;&#39;)/g,
             '<span class="sx-str">$1</span>')
    .replace(/(^|\n)(\s*#[^\n]*)/g, '$1<span class="sx-com">$2</span>')
    .replace(/\b(def|class|return|if|elif|else|for|while|try|except|finally|with|as|import|from|not|and|or|in|is|None|True|False|lambda|raise|pass|continue|break|yield|global|assert|del)\b/g,
             '<span class="sx-kw">$1</span>')
    .replace(/\b(\d+\.?\d*)\b/g, '<span class="sx-num">$1</span>');
}

async function renderSftCodeExplorer() {
  swTeardown();
  $('#pageTitle').textContent = 'SFT Code Explorer';
  $('#pageSub').textContent = 'the actual sftqc implementation · read-only';

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="sk-line"></div><div class="sk-line short"></div></div></div>`;

  let idx;
  try {
    idx = await api('/api/pipelines/sft/code');
  } catch (e) { return errorView(e); }

  if (!idx.files || !idx.files.length) {
    root.innerHTML = `<div class="content"><div class="content-inner">
      ${notConfiguredPanel('SFT Code Explorer',
        'No Python sources found under ' + (idx.projectPath || '~/soket/sftqc') +
        '. The explorer lists the real files the pipeline runs; it never invents code.',
        'sftqc')}
    </div></div>`;
    return;
  }

  // Deep link: #sft/code?f=<symbol or file hint>
  const hint = (() => {
    const m = (location.hash.match(/[?&]f=([^&]+)/) || [])[1];
    try { return m ? decodeURIComponent(m) : ''; } catch { return ''; }
  })();

  // Pick the initial file: hinted name, else pipeline.py, else first.
  let sel = idx.files.find(f => hint && (hint.includes(f.name) || f.name === hint))
         || idx.files[0];

  root.innerHTML = `<div class="content"><div class="content-inner sx-wrap">
    <div class="pa-toolbar">
      <div class="pa-toolbar-l">
        <button type="button" class="pa-btn" id="sxBackBtn" aria-label="Back to runs view">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
               stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M19 12H5"/><path d="m12 19-7-7 7-7"/></svg>
          Runs</button>
        <span class="pa-sep"></span>
        <a class="pa-open-standalone" href="/sft-pipeline-map" target="_blank"
           rel="noopener">Pipeline map ↗</a>
      </div>
      <div class="pa-toolbar-r">
        <span class="pa-demo-flag" title="Read-only view of the real source. Nothing is executed.">READ-ONLY · live source</span>
      </div>
    </div>
    <div class="sx-layout">
      <aside class="sx-side">
        <div class="sx-side-h">source files</div>
        <div class="sx-files" id="sxFiles"></div>
      </aside>
      <section class="sx-main">
        <div class="sx-head">
          <div>
            <div class="sx-file-name" id="sxFileName"></div>
            <div class="sx-file-path" id="sxFilePath"></div>
          </div>
          <div class="sx-file-meta" id="sxFileMeta"></div>
        </div>
        <div class="sx-syms" id="sxSyms"></div>
        <div class="sx-doc" id="sxDoc" hidden></div>
        <pre class="sx-src"><code id="sxSrc"></code></pre>
      </section>
    </div>
    <div class="pa-note-line">Served from <code>${esc(idx.projectPath)}</code> through
      <code>/api/pipelines/sft/code</code> · the pipeline's own CLI
      (<code>python -m sftqc.pipeline …</code>) remains the authoritative runner.</div>
  </div></div>`;

  $('#sxBackBtn').onclick = () => { location.hash = '#sft'; };

  const filesEl = $('#sxFiles');
  filesEl.innerHTML = idx.files.map(f =>
    `<button type="button" class="sx-file" data-f="${esc(f.name)}">
       <span class="sx-file-n">${esc(f.name)}</span>
       <span class="sx-file-l">${fmtInt(f.lines)} lines · ${f.symbols.length} sym</span>
     </button>`).join('');

  async function show(name) {
    const meta = idx.files.find(f => f.name === name);
    if (!meta) return;
    filesEl.querySelectorAll('.sx-file').forEach(b =>
      b.classList.toggle('sx-file-on', b.dataset.f === name));
    $('#sxFileName').textContent = meta.name;
    $('#sxFilePath').textContent = meta.path;
    $('#sxFileMeta').textContent =
      `${fmtInt(meta.lines)} lines · ${fmtInt(meta.bytes)} bytes · ${meta.symbols.length} top-level symbols`;
    // Symbols jump to their line.
    $('#sxSyms').innerHTML = meta.symbols.map(s =>
      `<button type="button" class="sx-sym" data-ln="${s.line}">
         <span class="sx-sym-k ${s.kind === 'class' ? 'sx-sym-class' : ''}">${s.kind}</span>
         <span class="sx-sym-n">${esc(s.name)}</span>
         <span class="sx-sym-l">:${s.line}</span>
       </button>`).join('');
    const docEl = $('#sxDoc');
    if (meta.doc) { docEl.hidden = false; docEl.textContent = meta.doc; }
    else docEl.hidden = true;

    const code = $('#sxSrc');
    code.innerHTML = `<div class="sk-line"></div>`;
    try {
      const res = await fetch('/api/pipelines/sft/code/file?name=' + encodeURIComponent(name),
                              { cache: 'no-store' });
      if (!res.ok) throw new Error('HTTP ' + res.status);
      const text = await res.text();
      code.innerHTML = sxHighlight(text);
    } catch (e) {
      code.innerHTML = `<span class="dim">// could not load ${esc(name)}: ${esc(e.message)}</span>`;
    }
  }

  filesEl.addEventListener('click', e => {
    const b = e.target.closest('.sx-file');
    if (b) show(b.dataset.f);
  });
  $('#sxSyms').addEventListener('click', e => {
    const b = e.target.closest('.sx-sym');
    if (!b) return;
    const ln = parseInt(b.dataset.ln, 10);
    const pre = $('.sx-src');
    if (pre && ln) {
      // Scroll by line height: the highlighted source is one flowing
      // <code> block, so per-line elements are not guaranteed.
      const lh = parseFloat(getComputedStyle(pre).lineHeight) || 20;
      pre.scrollTo({ top: Math.max(0, (ln - 6) * lh), behavior: 'smooth' });
    }
  });

  await show(sel.name);

  // If the deep link named a symbol, jump to it once the file is rendered.
  if (hint) {
    const sym = sel.symbols.find(s => hint.includes(s.name));
    if (sym) setTimeout(() => {
      const pre = $('.sx-src');
      const lh = parseFloat(getComputedStyle(pre).lineHeight) || 20;
      pre.scrollTo({ top: Math.max(0, (sym.line - 6) * lh), behavior: 'smooth' });
    }, 120);
  }
}

/* ============================================================
   PRETRAINING DATATROVE WORKSPACE (route: pretrain)
   Real source: /api/pipelines/pretrain → codepipe stage markers.
   Stage cards come from the stages that actually exist on disk; docs/bytes
   are the pipeline's own accounting, never re-derived from datasets.
   ============================================================ */
async function renderPretrainPipeline() {
  // Sub-route: the interactive architecture canvas.
  if (paSubRoute() === 'architecture') return renderPretrainArchitecture();
  // Sub-route: the navigable architecture report.
  if (paSubRoute() === 'report') return renderPretrainReport();
  paTeardown();

  $('#pageTitle').textContent = 'Pretraining DataTrove';
  $('#pageSub').textContent = 'large-scale code corpus preparation · staged by codepipe';

  const root = $('#viewRoot');
  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="sk-line"></div><div class="sk-line short"></div></div></div>`;

  let s;
  try {
    s = await api('/api/pipelines/pretrain');
  } catch (e) { return errorView(e); }

  if (s.integration === 'not_configured') {
    root.innerHTML = `<div class="content"><div class="content-inner">
      ${notConfiguredPanel('Pretraining DataTrove', s.unavailableReason, 'codepipe')}
      <div class="pa-entry">
        <div>
          <div class="pa-entry-t">Pretraining architecture</div>
          <div class="pa-entry-s">The pipeline's documented design — collection, extraction,
            filtering, preprocessing, classification and tokenization — as an interactive map.
            No runs are needed to explore it.</div>
        </div>
        <a class="pa-entry-btn" href="#pretrain/architecture">Open architecture map
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
               stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg></a>
      </div>
    </div>` + `<div class="pa-entry">
      <div>
        <div class="pa-entry-t">Architecture report</div>
        <div class="pa-entry-s">The complete navigable reference — every phase, stage and component explained, with search, quick-read summaries and codepipe cross-references.</div>
      </div>
      <a class="pa-entry-btn" href="#pretrain/report">Open report
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
             stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg></a>
    </div>
    </div></div>`;
    return;
  }

  const t = s.totals || {};
  const kpis = [
    ['stages', fmtInt(t.stages), 'pipeline stages with completed tasks', 'var(--amber)'],
    ['tasks done', fmtInt(t.tasksDone), 'finished task markers across stages', 'var(--blue)'],
    ['docs kept', fmtInt(t.finalKeptDocs), 'by the final discovered stage', 'var(--acc)'],
    ['est. tokens', t.estTokensKept != null ? fmtTokensEst(t.estTokensKept) : '—',
      'kept bytes ÷ 3.5 (codepipe\'s own estimate)', 'var(--violet)'],
  ];

  let stageHtml;
  if (!(s.stages || []).length) {
    stageHtml = `<div class="empty-note">No stage outputs discovered. Stages appear here as
      <code>codepipe</code> writes them under the output root
      (${esc(s.outputRoot || 'the workspace root')}).</div>`;
  } else {
    stageHtml = `<div class="stage-flow">` + s.stages.map((x, i) => {
      const keepPct = x.docsIn ? (100 * x.docsKept / x.docsIn) : 0;
      const bytePct = x.bytesIn ? (100 * x.bytesKept / x.bytesIn) : 0;
      const reasons = (x.removalReasons || []).slice(0, 4).map(r =>
        `<div class="reason-row"><span class="reason-k">${esc(r.reason)}</span>
         <span class="reason-v">${fmtInt(r.docs)} docs · ${fmtBytes(r.bytes, 0)}</span></div>`).join('');
      return `<div class="stage-card">
        <div class="stage-head">
          <span class="stage-num">${String(i + 1).padStart(2, '0')}</span>
          <div><div class="stage-name">${esc(x.name)}</div>
            <div class="stage-id">${esc(x.id)} · ${x.tasksDone} task${x.tasksDone === 1 ? '' : 's'} · ${x.languages} lang${x.languages === 1 ? '' : 's'}</div></div>
        </div>
        <div class="stage-bars">
          <div class="stage-bar-row"><span>docs</span>
            <div class="lang-bar"><i style="width:${keepPct.toFixed(1)}%"></i></div>
            <b>${keepPct.toFixed(1)}%</b></div>
          <div class="stage-bar-row"><span>bytes</span>
            <div class="lang-bar"><i style="width:${bytePct.toFixed(1)}%"></i></div>
            <b>${bytePct.toFixed(1)}%</b></div>
        </div>
        <div class="stage-stats">
          <div><span class="dim">in</span> ${fmtInt(x.docsIn)} docs · ${fmtBytes(x.bytesIn, 0)}</div>
          <div><span class="dim">kept</span> ${fmtInt(x.docsKept)} docs · ${fmtBytes(x.bytesKept, 0)}</div>
          <div><span class="dim">est. tokens kept</span> ${fmtTokensEst(x.estTokensKept)}</div>
        </div>
        ${reasons ? `<div class="stage-reasons"><div class="dim">top removal reasons</div>${reasons}</div>` : ''}
        <div class="stage-path">${esc(x.path)}</div>
      </div>`;
    }).join('') + `</div>
    <div class="panel-sub" style="padding:0 4px 14px">Documents flow through every stage, so
      counts describe each stage's own throughput — they are deliberately not summed across stages.</div>`;
  }

  root.innerHTML = `<div class="content"><div class="content-inner">
    <div class="kpis">${kpis.map(x =>
      `<div class="kpi" style="--kcol:${x[3]}"><div class="kpi-k">${x[0]}</div>
       <div class="kpi-v">${x[1]}</div><div class="kpi-s">${x[2]}</div></div>`).join('')}
    </div>
    <div class="pa-entry">
      <div>
        <div class="pa-entry-t">Pretraining architecture</div>
        <div class="pa-entry-s">Explore the full documented pipeline — collection, extraction,
          filtering, preprocessing, classification and tokenization — on an interactive canvas.</div>
      </div>
      <a class="pa-entry-btn" href="#pretrain/architecture">Open architecture map
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
             stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg></a>
    </div>

    <div class="pa-entry">
      <div>
        <div class="pa-entry-t">Architecture report</div>
        <div class="pa-entry-s">The complete navigable reference — every phase, stage and
          component explained, with search, quick-read summaries and codepipe cross-references.</div>
      </div>
      <a class="pa-entry-btn" href="#pretrain/report">Open report
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
             stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg></a>
    </div>

    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Pipeline stages</div>
      <div class="panel-sub">discovered from disk · aggregated from each stage's _done task markers</div>
    </div></div>
      <div style="padding:16px 18px">${stageHtml}</div></div>

    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Execution</div>
      <div class="panel-sub">read-only view · jobs are submitted through Slurm arrays or local workers</div>
    </div></div>
      <div style="padding:14px 18px;font-family:var(--mono);font-size:11.5px;line-height:2;color:var(--ink-dim)">
        <div><span class="dim"># inventory the corpus (writes manifest + sample + stats.json)</span></div>
        <div>python -m codepipe.inventory --input &lt;DATA&gt; --out $OUT/00_inventory --workers 64</div>
        <div style="height:8px"></div>
        <div><span class="dim"># stage 1 filter as a Slurm array (resume-safe via _done markers)</span></div>
        <div>sbatch --array=0-$((N-1))%500 --cpus-per-task=1 --mem=8G \\</div>
        <div>&nbsp;&nbsp;--wrap "python -m codepipe.filter --inventory $OUT/00_inventory --out $OUT/01_filter --num-tasks $N"</div>
        <div style="height:8px"></div>
        <div><span class="dim"># exact dedup: hashes → select → apply (chain with --dependency)</span></div>
        <div>python -m codepipe.exact_dedup --input $OUT/01_filter/kept --out $OUT/02_exact --step hashes</div>
        <div style="height:8px"></div>
        <div><span class="dim"># regenerate the human-readable report the dashboard mirrors</span></div>
        <div>python -m codepipe.report $OUT/01_filter</div>
        <div style="height:8px"></div>
        <div><span class="dim"># project + output locations on this machine</span></div>
        <div>${esc(s.projectPath || '')}${s.outputRoot ? '<br>' + esc(s.outputRoot) : ''}</div>
      </div></div>
  </div></div>`;
  $('#refreshNote').textContent = 'updated ' +
    new Date().toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

/* Honest "not configured" panel shared by both pipeline workspaces. */
function notConfiguredPanel(title, reason, project) {
  return `<div class="panel"><div class="panel-h"><div>
      <div class="panel-t">${esc(title)} — not configured on this machine</div>
      <div class="panel-sub">integration state: not configured</div>
    </div></div>
    <div style="padding:22px 24px">
      <div style="font-family:var(--mono);font-size:12px;color:var(--ink-dim);line-height:1.8;max-width:760px">
        ${esc(reason || '')}
      </div>
      <div style="margin-top:16px;font-family:var(--mono);font-size:11px;color:var(--ink-faint);line-height:1.9">
        This workspace reads the artifacts <code>${esc(project)}</code> writes
        (report.json, _done task markers). It never launches jobs, scans datasets,
        or invents status. Once the project and its outputs exist at the paths above,
        the workspace populates automatically.
      </div>
    </div></div>`;
}

function errorView(err) {
  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <div class="error-banner"><div>
      <div class="error-t">Could not load data</div>
      <div class="error-s">${esc(err.message || String(err))}</div>
      <button class="retry-btn" onclick="render()">retry</button>
    </div></div></div></div>`;
}

/* ============================================================
   CHAT / PLAYGROUND
   ============================================================ */
async function renderChat() {
  $('#pageTitle').textContent = 'Playground';
  $('#pageSub').textContent = 'chat with the real Loop inference engine';

  $('#viewRoot').innerHTML = `
    <div class="chat-layout">
      <aside class="conv-list">
        <div class="conv-head"><h3>Conversations</h3>
          <button class="new-chat-btn" id="newChatBtn">+ new</button></div>
        <div class="conv-scroll" id="convList"></div>
      </aside>
      <div class="chat-main">
        <div class="chat-scroll" id="chatScroll"><div class="chat-inner" id="chatInner"></div></div>
        <div class="composer-wrap">
          <div class="composer">
            <div class="composer-bar">
              <textarea id="chatInput" placeholder="Message the model…  (Enter to send, Shift+Enter for newline)" rows="1"></textarea>
              <button class="send-btn" id="sendBtn">Send</button>
              <button class="stop-btn hidden" id="stopBtn">Stop</button>
            </div>
            <div class="composer-meta">
              <select class="model-sel" id="modelSel"></select>
              <button class="adv-toggle" id="advToggle">⚙ parameters</button>
              <span id="paramSummary"></span>
            </div>
            <div class="adv-panel" id="advPanel">
              <div class="adv-field"><label>temperature</label>
                <input id="pTemp" type="number" step="0.1" min="0" max="2" placeholder="default">
                <span class="param-hint">0.0 – 2.0</span></div>
              <div class="adv-field"><label>max tokens</label>
                <input id="pMax" type="number" step="1" min="1" max="200000" placeholder="default">
                <span class="param-hint">1 – 200000</span></div>
              <div class="adv-field"><label>reasoning</label>
                <select id="pReason"><option value="">default</option></select>
                <span class="param-hint">only for reasoning models</span></div>
            </div>
          </div>
        </div>
      </div>
    </div>`;

  let modelsBody, paramsBody;
  try {
    [modelsBody, paramsBody] = await Promise.all([api('/api/models'), api('/api/models/params')]);
  } catch (e) {
    $('#chatInner').innerHTML = `<div class="empty"><div class="empty-t">Inference unavailable</div>
      <div class="empty-s">${esc(e.message)}</div></div>`;
    return;
  }
  S.models = modelsBody.models || [];
  S.paramsMeta = paramsBody;

  const sel = $('#modelSel');
  sel.innerHTML = S.models.length
    ? S.models.map(m => `<option value="${esc(m.provider)}/${esc(m.id)}">${esc(m.provider)} / ${esc(m.name)}</option>`).join('')
    : '<option value="">no models configured</option>';
  if (!S.models.length) {
    $('#chatInner').innerHTML = `<div class="empty"><div class="empty-t">No models available</div>
      <div class="empty-s">Connect a provider first: run <b>loop</b> and use /login,<br>
      or set SOKET_API_KEY / OPENROUTER_API_KEY / OPENAI_API_KEY.</div></div>`;
    $('#sendBtn').disabled = true;
  }
  const savedModel = localStorage.getItem('loop-console-model');
  if (savedModel && S.models.some(m => `${m.provider}/${m.id}` === savedModel)) sel.value = savedModel;
  sel.onchange = () => localStorage.setItem('loop-console-model', sel.value);

  const reasonSel = $('#pReason');
  (S.paramsMeta.reasoningLevels || []).forEach(l => {
    const o = el('option', '', l); o.value = l; reasonSel.appendChild(o);
  });

  $('#newChatBtn').onclick = () => newConversation();
  $('#advToggle').onclick = () => $('#advPanel').classList.toggle('open');

  const tempInput = $('#pTemp'), maxInput = $('#pMax');
  const validateParams = () => {
    let ok = true;
    const t = tempInput.value.trim(), m = maxInput.value.trim();
    tempInput.classList.remove('invalid'); maxInput.classList.remove('invalid');
    if (t !== '') { const v = parseFloat(t); if (isNaN(v) || v < 0 || v > 2) { tempInput.classList.add('invalid'); ok = false; } }
    if (m !== '') { const v = parseInt(m, 10); if (isNaN(v) || v < 1 || v > 200000) { maxInput.classList.add('invalid'); ok = false; } }
    $('#sendBtn').disabled = !ok || !S.models.length;
    const parts = [];
    if (t !== '') parts.push('T=' + t);
    if (m !== '') parts.push('max=' + m);
    const r = reasonSel.value; if (r) parts.push('effort=' + r);
    $('#paramSummary').textContent = parts.join(' · ');
    return ok;
  };
  tempInput.oninput = validateParams; maxInput.oninput = validateParams; reasonSel.onchange = validateParams;
  validateParams();

  await loadConversations();
  bindComposer();
  if (!S.activeConv) newConversation(false);
  renderConversationList();
  renderMessages();
}

async function loadConversations() {
  try {
    const body = await api('/api/conversations');
    S.conversations = body.conversations || [];
  } catch { S.conversations = []; }
}

function newConversation(rerender = true) {
  S.activeConv = { id: null, title: 'New conversation', messages: [] };
  if (rerender) { renderConversationList(); renderMessages(); $('#chatInput')?.focus(); }
}

function renderConversationList() {
  const list = $('#convList'); if (!list) return;
  list.innerHTML = '';
  if (!S.activeConv || !S.activeConv.id) {
    const item = el('div', 'conv-item active',
      '<div class="conv-title">New conversation</div><div class="conv-meta">not saved yet</div>');
    item.onclick = () => newConversation();
    list.appendChild(item);
  }
  if (!S.conversations.length) {
    list.appendChild(el('div', '',
      '<div style="padding:10px;font-family:var(--mono);font-size:10.5px;color:var(--ink-faint)">No saved conversations</div>'));
  }
  for (const c of S.conversations) {
    const item = el('div', 'conv-item' + (S.activeConv && S.activeConv.id === c.conversationId ? ' active' : ''));
    item.innerHTML = `<span class="conv-del" title="delete">×</span>
      <div class="conv-title">${esc(c.title)}</div>
      <div class="conv-meta">${fmtTime(c.updatedAtMs || c.createdAtMs)} · ${c.messageCount} msgs</div>`;
    item.onclick = async e => {
      if (e.target.classList.contains('conv-del')) { await deleteConversation(c.conversationId); return; }
      await openConversation(c.conversationId);
    };
    list.appendChild(item);
  }
}

async function openConversation(id) {
  try {
    const body = await api('/api/conversations/' + encodeURIComponent(id));
    S.activeConv = {
      id: body.conversation.conversationId,
      title: body.conversation.title,
      messages: body.messages || [],
    };
    renderConversationList();
    renderMessages();
  } catch (e) { toast(e.message); }
}

async function deleteConversation(id) {
  if (!confirm('Delete this conversation?')) return;
  try {
    await api('/api/conversations/' + encodeURIComponent(id), { method: 'DELETE' });
    if (S.activeConv && S.activeConv.id === id) newConversation(false);
    await loadConversations();
    renderConversationList(); renderMessages();
    toast('conversation deleted');
  } catch (e) { toast(e.message); }
}

function renderMessages() {
  const inner = $('#chatInner'); if (!inner) return;
  inner.innerHTML = '';
  const conv = S.activeConv;
  if (!conv || !conv.messages.length) {
    // The empty state greets the user as Loop (the assistant), never as the
    // underlying model. The model selector stays independent below the composer.
    inner.innerHTML = `<div class="empty" style="padding-top:80px"><div class="empty-t">Hello! I'm Loop</div>
      <div class="empty-s" style="max-width:520px">
        I'm your intelligent assistant powered by <b>Soket.ai</b>. I'm here to help
        you with coding, research, data analysis, debugging, and problem-solving.<br>
        How can I help you today?<br>
        <span style="opacity:.75">Messages are sent to the real inference engine and
        every request is traced and durably persisted.</span>
      </div></div>`;
    return;
  }
  for (const m of conv.messages) {
    inner.appendChild(messageNode(m.role, m.content, m.status, m.errorMessage, m.createdAtMs, m.model));
  }
  scrollChat(true);
}

function messageNode(role, content, status, error, ts, model) {
  const wrap = el('div', 'msg ' + role);
  const streaming = status === 'streaming';
  wrap.innerHTML = `
    <div class="msg-head">
      <span class="msg-role">${role}</span>
      ${model ? `<span class="msg-time">${esc(model)}</span>` : ''}
      <span class="msg-time">${fmtTime(ts)}</span>
      <div class="msg-actions"><button class="icon-btn copy-msg" title="copy message">⧉</button></div>
    </div>
    <div class="msg-body">${mdToHtml(content)}${streaming ? '<span class="streaming-cursor"></span>' : ''}</div>
    ${status === 'error' && error ? `<div class="msg-error">${esc(error)}</div>` : ''}
    ${status === 'cancelled' ? '<div class="msg-cancelled">generation cancelled — partial output shown</div>' : ''}`;
  wrap.querySelector('.copy-msg').onclick = () => {
    navigator.clipboard.writeText(content || '').then(() => toast('copied'));
  };
  return wrap;
}

function scrollChat(force) {
  const sc = $('#chatScroll'); if (!sc) return;
  const nearBottom = sc.scrollHeight - sc.scrollTop - sc.clientHeight < 120;
  if (force || nearBottom) sc.scrollTop = sc.scrollHeight;
}

function bindComposer() {
  const input = $('#chatInput'), send = $('#sendBtn'), stop = $('#stopBtn');
  input.onkeydown = e => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send.click(); }
  };
  input.oninput = () => {
    input.style.height = 'auto';
    input.style.height = Math.min(input.scrollHeight, 200) + 'px';
  };
  send.onclick = () => sendMessage();
  stop.onclick = () => stopGeneration();
}

function collectParams() {
  const t = $('#pTemp').value.trim(), m = $('#pMax').value.trim(), r = $('#pReason').value;
  const params = {};
  if (t !== '') params.temperature = parseFloat(t);
  if (m !== '') params.maxTokens = parseInt(m, 10);
  if (r) params.reasoning = r;
  return params;
}

async function sendMessage() {
  const input = $('#chatInput'), send = $('#sendBtn'), stop = $('#stopBtn');
  const text = input.value.trim();
  if (!text || S.streaming) return;
  const modelVal = $('#modelSel').value;
  if (!modelVal) { toast('no model selected'); return; }
  const idx = modelVal.indexOf('/');
  const provider = modelVal.slice(0, idx), model = modelVal.slice(idx + 1);

  const conv = S.activeConv || (S.activeConv = { id: null, title: 'New conversation', messages: [] });
  conv.messages.push({ role: 'user', content: text, status: 'complete', createdAtMs: Date.now() });
  const assistantMsg = { role: 'assistant', content: '', status: 'streaming', createdAtMs: Date.now(), model };
  conv.messages.push(assistantMsg);
  input.value = ''; input.style.height = 'auto';
  renderMessages();

  const controller = new AbortController();
  S.streaming = { abort: controller, requestId: null };
  send.classList.add('hidden'); stop.classList.remove('hidden');
  $('#pageSub').textContent = 'generating…';

  let startedIds = null;
  try {
    const res = await fetch('/api/chat', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        conversationId: conv.id,
        message: text,
        provider, model,
        params: collectParams(),
      }),
      signal: controller.signal,
    });
    if (!res.ok) {
      const body = await res.json().catch(() => ({}));
      throw new Error(body.error || ('HTTP ' + res.status));
    }
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buf = '';
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buf += decoder.decode(value, { stream: true });
      let sep;
      while ((sep = buf.indexOf('\n\n')) >= 0) {
        const chunk = buf.slice(0, sep); buf = buf.slice(sep + 2);
        for (const line of chunk.split('\n')) {
          if (!line.startsWith('data:')) continue;
          const payload = line.slice(5).trim();
          if (!payload) continue;
          let ev; try { ev = JSON.parse(payload); } catch { continue; }
          if (ev.type === 'started') startedIds = ev;
          handleChatEvent(ev, assistantMsg);
        }
      }
    }
  } catch (err) {
    if (err.name === 'AbortError') {
      assistantMsg.status = 'cancelled';
    } else {
      assistantMsg.status = 'error';
      assistantMsg.errorMessage = err.message;
    }
  } finally {
    S.streaming = null;
    send.classList.remove('hidden'); stop.classList.add('hidden');
    $('#pageSub').textContent = 'chat with the real Loop inference engine';
    renderMessages();
    await loadConversations();
    renderConversationList();
    if (startedIds && startedIds.conversationId) S.activeConv.id = startedIds.conversationId;
  }
}

function handleChatEvent(ev, assistantMsg) {
  switch (ev.type) {
    case 'started':
      if (S.streaming) S.streaming.requestId = ev.requestId;
      break;
    case 'textDelta':
      assistantMsg.content += ev.delta;
      updateStreamingMessage(assistantMsg);
      break;
    case 'thinkingDelta':
      assistantMsg.thinking = (assistantMsg.thinking || '') + ev.delta;
      updateStreamingMessage(assistantMsg);
      break;
    case 'done':
      assistantMsg.content = ev.text;
      assistantMsg.status = 'complete';
      break;
    case 'failed':
      assistantMsg.content = ev.partialText || assistantMsg.content;
      assistantMsg.status = 'error';
      assistantMsg.errorMessage = ev.error;
      break;
    case 'cancelled':
      assistantMsg.content = ev.partialText || assistantMsg.content;
      assistantMsg.status = 'cancelled';
      break;
  }
}

function updateStreamingMessage(msg) {
  // Update only the streaming node — no full re-render, no DOM accumulation.
  const nodes = document.querySelectorAll('#chatInner .msg.assistant');
  const node = nodes[nodes.length - 1];
  if (!node) return;
  const body = node.querySelector('.msg-body');
  body.innerHTML = mdToHtml(msg.content) +
    (msg.thinking ? `<div class="thinking-block">${esc(msg.thinking)}</div>` : '') +
    '<span class="streaming-cursor"></span>';
  scrollChat(false);
}

async function stopGeneration() {
  if (!S.streaming) return;
  // Cancel the real underlying request through the backend.
  if (S.streaming.requestId) {
    try {
      await api('/api/chat/' + encodeURIComponent(S.streaming.requestId) + '/cancel', { method: 'POST' });
    } catch { /* pump finalizes on disconnect anyway */ }
  }
  S.streaming.abort.abort();
}

/* ============================================================
   TRACES
   ============================================================ */
function statusPill(status) {
  const map = {
    success: ['ok', 'success'], error: ['bad', 'error'],
    cancelled_user: ['warn', 'cancelled'], cancelled_client: ['warn', 'client left'],
    interrupted: ['inc', 'interrupted'], running: ['inc', 'running'],
  };
  const [cls, label] = map[status] || ['inc', status];
  return `<span class="pill ${cls}">${label}</span>`;
}

async function renderTraces() {
  $('#pageTitle').textContent = 'Traces';
  $('#pageSub').textContent = 'durable record of every inference execution';
  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <div class="filters">
      <span class="filter-label">search</span>
      <div class="search-box"><input id="traceQ" placeholder="trace id, operation, model…" value="${esc(S.traces.q)}"></div>
      <span class="filter-label">status</span>
      <div class="seg" id="traceStatus">${['all','success','error','cancelled_user','cancelled_client','interrupted']
        .map(s => `<button data-s="${s}" class="${S.traces.status===s?'on':''}">${s.replace(/_/g,' ')}</button>`).join('')}</div>
      <span class="filter-label">sort</span>
      <div class="seg" id="traceSort">
        <button data-v="started" class="${S.traces.sort==='started'?'on':''}">time</button>
        <button data-v="duration" class="${S.traces.sort==='duration'?'on':''}">duration</button>
      </div>
      <button class="adv-toggle" id="traceRefresh">↻ refresh</button>
    </div>
    <div class="panel">
      <div class="table-scroll"><table>
        <thead><tr>
          <th>trace</th><th>operation</th><th>model</th><th>status</th>
          <th>started</th><th class="sortable" id="durSort">duration ⇅</th>
          <th>tokens in</th><th>tokens out</th><th>ttft</th>
        </tr></thead>
        <tbody id="traceBody"></tbody>
      </table></div>
      <div class="tbl-foot">
        <span class="tbl-count" id="traceCount"></span>
        <div class="pager" id="tracePager"></div>
      </div>
    </div>
  </div></div>`;

  $('#traceQ').oninput = debounce(async e => {
    S.traces.q = e.target.value; S.traces.page = 1; await loadTraces();
  }, 250);
  $('#traceStatus').querySelectorAll('button').forEach(b => b.onclick = async () => {
    S.traces.status = b.dataset.s; S.traces.page = 1;
    $('#traceStatus').querySelectorAll('button').forEach(x => x.classList.toggle('on', x === b));
    await loadTraces();
  });
  $('#traceSort').querySelectorAll('button').forEach(b => b.onclick = async () => {
    S.traces.sort = b.dataset.v;
    $('#traceSort').querySelectorAll('button').forEach(x => x.classList.toggle('on', x === b));
    await loadTraces();
  });
  $('#traceRefresh').onclick = () => loadTraces();
  $('#durSort').onclick = async () => {
    S.traces.dir = S.traces.dir === 'asc' ? 'desc' : 'asc';
    await loadTraces();
  };
  await loadTraces();
}

async function loadTraces() {
  const params = new URLSearchParams({
    page: S.traces.page, per_page: S.traces.perPage,
    sort: S.traces.sort, dir: S.traces.dir,
  });
  if (S.traces.q) params.set('q', S.traces.q);
  if (S.traces.status !== 'all') params.set('status', S.traces.status);
  let body;
  try { body = await api('/api/traces?' + params); }
  catch (e) { $('#traceBody').innerHTML = `<tr><td colspan="9" style="color:var(--red)">${esc(e.message)}</td></tr>`; return; }
  Object.assign(S.traces, { items: body.items, total: body.total, pages: body.pages });
  renderTraceRows($('#traceBody'), body.items, true, 9);
  $('#traceCount').textContent = `showing ${(body.page-1)*body.perPage+1}–${Math.min(body.page*body.perPage, body.total)} of ${fmtInt(body.total)}`;
  renderPager($('#tracePager'), body, p => { S.traces.page = p; loadTraces(); });
}

function renderTraceRows(tbody, items, clickable, colspan) {
  tbody.innerHTML = '';
  if (!items.length) {
    tbody.innerHTML = `<tr><td colspan="${colspan}"><div class="empty"><div class="empty-t">No traces</div>
      <div class="empty-s">Send a message in the Playground — every request is traced.</div></div></td></tr>`;
    return;
  }
  for (const t of items) {
    const tr = el('tr');
    tr.innerHTML = `
      <td class="mono dim" title="${esc(t.traceId)}">${esc(String(t.traceId).slice(0, 8))}…</td>
      <td class="mono dim">${esc(t.operation)}</td>
      <td class="mono dim">${esc(t.model || '—')}</td>
      <td>${statusPill(t.status)}</td>
      <td class="mono faint">${fmtTime(t.startedAtMs)}</td>
      <td class="mono dim">${fmtMs(t.durationMs)}</td>
      <td class="mono dim">${t.inputTokens != null ? fmtInt(t.inputTokens) : '—'}</td>
      <td class="mono dim">${t.outputTokens != null ? fmtInt(t.outputTokens) : '—'}</td>
      <td class="mono dim">${fmtMs(t.ttftMs)}</td>`;
    if (clickable) tr.onclick = () => openTraceDetail(t.traceId);
    tbody.appendChild(tr);
  }
}

function renderPager(container, body, go) {
  container.innerHTML = '';
  const mk = (label, page, opts = {}) => {
    const b = el('button', opts.cls || '', String(label));
    b.disabled = !!opts.disabled;
    b.onclick = () => go(page);
    container.appendChild(b);
  };
  mk('‹', body.page - 1, { disabled: body.page <= 1 });
  const win = 2;
  for (let p = 1; p <= body.pages; p++) {
    if (p !== 1 && p !== body.pages && Math.abs(p - body.page) > win) {
      if (p === 2 || p === body.pages - 1) container.appendChild(el('span', 'faint mono', '…'));
      continue;
    }
    mk(p, p, { cls: p === body.page ? 'cur' : '' });
  }
  mk('›', body.page + 1, { disabled: body.page >= body.pages });
}

async function openTraceDetail(traceId) {
  let detail;
  try { detail = await api('/api/traces/' + encodeURIComponent(traceId)); }
  catch (e) { toast(e.message); return; }
  const t = detail.trace;

  // Phase 2: captured conversation content (admin-only endpoint). A 403
  // means this caller is not an administrator; the page then renders the
  // metadata-only view exactly as before.
  let content = null, contentBlocked = false;
  try {
    content = await api('/api/admin/traces/' + encodeURIComponent(traceId) + '/content');
  } catch (e) {
    contentBlocked = true;
  }

  $('#pageTitle').textContent = 'Trace ' + String(t.traceId).slice(0, 8) + '…';
  $('#pageSub').textContent = `${t.operation} · ${t.model || 'unknown model'}`;

  const dm = (k, v) => `<div class="dm"><div class="dm-k">${k}</div><div class="dm-v">${v}</div></div>`;
  let paramsPretty = '—';
  if (t.paramsJson) {
    try { paramsPretty = esc(JSON.stringify(JSON.parse(t.paramsJson), null, 1)); } catch { paramsPretty = esc(t.paramsJson); }
  }

  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <button class="adv-toggle" id="backToTraces" style="margin-bottom:12px">← all traces</button>
    <div class="detail-grid">
      <div>
        <div class="panel"><div class="panel-h"><div>
          <div class="panel-t">Metadata</div>
          <div class="panel-sub">recorded measurements only</div>
        </div></div>
          <div style="padding:15px 18px">
            ${dm('status', statusPill(t.status))}
            ${dm('trace id', `<span class="dim">${esc(t.traceId)}</span>`)}
            ${dm('session id', t.sessionId ? `<span class="dim">${esc(t.sessionId)}</span>` : '—')}
            ${dm('request id', t.requestId ? `<span class="dim">${esc(t.requestId)}</span>` : '—')}
            ${dm('model', esc(t.model || '—'))}
            ${dm('provider', esc(t.provider || '—'))}
            ${dm('started', fmtTimeFull(t.startedAtMs))}
            ${dm('ended', t.endedAtMs ? fmtTimeFull(t.endedAtMs) : '—')}
            ${dm('duration', fmtMs(t.durationMs))}
            ${dm('input tokens', t.inputTokens != null ? fmtInt(t.inputTokens) : 'not reported')}
            ${dm('output tokens', t.outputTokens != null ? fmtInt(t.outputTokens) : 'not reported')}
            ${dm('time to first token', t.ttftMs != null ? fmtMs(t.ttftMs) : 'not measured')}
            ${dm('parameters', `<pre style="white-space:pre-wrap;background:var(--bg);border:1px solid var(--line);border-radius:8px;padding:8px 10px;font-size:10.5px">${paramsPretty}</pre>`)}
            ${t.errorMessage ? dm('error', `<span style="color:var(--red)">${esc(t.errorMessage)}</span>`) : ''}
          </div></div>

        <div class="panel"><div class="panel-h"><div>
          <div class="panel-t">Token usage</div>
          <div class="panel-sub">as reported by the model runtime</div>
        </div></div>
          <div style="padding:15px 18px">
            ${dm('input tokens', t.inputTokens != null ? fmtInt(t.inputTokens) : 'Not reported')}
            ${dm('output tokens', t.outputTokens != null ? fmtInt(t.outputTokens) : 'Not reported')}
            ${dm('total tokens', (t.inputTokens != null || t.outputTokens != null) ? fmtInt((t.inputTokens || 0) + (t.outputTokens || 0)) : 'Not reported')}
            ${dm('time to first token', t.ttftMs != null ? fmtMs(t.ttftMs) : 'Not reported')}
            ${dm('generation duration', t.durationMs != null ? fmtMs(t.durationMs) : 'Not reported')}
          </div></div>

        <div class="panel"><div class="panel-h"><div>
          <div class="panel-t">Span timeline</div>
          <div class="panel-sub">actual span timestamps — not derived from request latency</div>
        </div></div>
          <div class="waterfall" id="waterfall"></div>
        </div>
        <div class="panel"><div class="panel-h"><div>
          <div class="panel-t">Span tree</div>
          <div class="panel-sub">parent-child relationships as recorded</div>
        </div></div>
          <div class="span-tree" id="spanTree"></div>
        </div>
      </div>

      <div>
        ${contentSection(content, contentBlocked)}
        ${turnsSection(content)}
        ${toolsSection(content)}
        ${errorsSection(content, t)}
        ${rawEventsSection(detail.events)}
      </div>
    </div>
  </div></div>`;

  $('#backToTraces').onclick = () => go('traces');
  renderWaterfall(detail.spans, t);
  renderSpanTree(detail.spans, t);
  bindTraceDetailActions(content);
}

/* ---------- Phase 2: conversation / turns / tools / errors / raw ---------- */

function copyBtn(text, label) {
  return `<button class="icon-btn copy-inline" data-copy="${esc(text)}" title="copy ${label}">⧉</button>`;
}

function contentSection(content, blocked) {
  if (blocked) {
    return `<div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Conversation</div>
      <div class="panel-sub">administrator access required to view captured content</div>
    </div></div>
      <div class="empty" style="padding:34px"><div class="empty-t">Content restricted</div>
      <div class="empty-s">Sign in as an administrator to inspect the complete prompt and response history.</div></div>
    </div>`;
  }
  const conv = (content && content.conversation) || [];
  if (!conv.length) {
    return `<div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Conversation</div>
      <div class="panel-sub">no captured messages on this trace</div>
    </div></div>
      <div class="empty" style="padding:34px"><div class="empty-t">No conversation captured</div>
      <div class="empty-s">Run a prompt through the Loop CLI agent with content capture enabled (Dashboard → Settings → capture content).</div></div>
    </div>`;
  }
  const rows = conv.map(m => {
    const kind = m.kind === 'user' ? 'user' : (m.kind === 'assistant_partial' ? 'assistant partial' : 'assistant');
    const cls = m.kind === 'user' ? 'tc-user' : 'tc-assistant';
    const text = m.text || '';
    const body = text
      ? `<div class="tc-body">${mdToHtml(text)}</div>`
      : '<div class="tc-body faint">no content captured for this message</div>';
    return `<div class="tc ${cls}">
      <div class="tc-head">
        <span class="tc-kind">${kind}</span>
        ${m.turn != null ? `<span class="tc-meta">turn ${m.turn}</span>` : ''}
        <span class="tc-meta">${fmtTimeFull(m.atMs)}</span>
        <div class="msg-actions">${text ? copyBtn(text, kind) : ''}</div>
      </div>
      ${body}
    </div>`;
  }).join('');
  return `<div class="panel"><div class="panel-h"><div>
    <div class="panel-t">Conversation</div>
    <div class="panel-sub">${conv.length} captured messages · chronological</div>
  </div>
  <input id="convSearch" class="search-box" placeholder="search conversation…" style="width:180px;font-size:11px;padding:5px 9px"></div>
    <div class="tc-list" id="convList">${rows}</div>
  </div>`;
}

function turnsSection(content) {
  const turns = (content && content.turns) || [];
  if (!turns.length) return '';
  const rows = turns.map(tn => `
    <div class="event-row">
      <span class="event-ts">${tn.startMs ? fmtTimeFull(tn.startMs) : '—'}</span>
      <span class="event-name">turn ${tn.turn ?? '?'}</span>
      ${statusPill(tn.status)}
      <span class="event-data">start ${tn.startMs ? fmtTimeFull(tn.startMs) : '—'} · end ${tn.endMs ? fmtTimeFull(tn.endMs) : '—'} · ${fmtMs(tn.durationMs)}</span>
    </div>`).join('');
  return `<div class="panel"><div class="panel-h"><div>
    <div class="panel-t">Turn details</div>
    <div class="panel-sub">${turns.length} turns · start / end / duration / status</div>
  </div></div>
    <div class="events-list">${rows}</div>
  </div>`;
}

function toolsSection(content) {
  const tools = (content && content.tools) || [];
  if (!tools.length) return '';
  const rows = tools.map(tc => {
    const args = tc.arguments != null
      ? `<pre class="tc-pre">${esc(typeof tc.arguments === 'string' ? tc.arguments : JSON.stringify(tc.arguments, null, 1))}</pre>`
      : '<div class="faint" style="font-size:11px">arguments not captured</div>';
    const out = tc.output != null
      ? `<pre class="tc-pre">${esc(tc.output)}</pre>`
      : '<div class="faint" style="font-size:11px">output not captured</div>';
    return `<div class="tc tool-card">
      <div class="tc-head">
        <span class="tc-kind">tool · ${esc(tc.tool || '?')}</span>
        ${tc.turn != null ? `<span class="tc-meta">turn ${tc.turn}</span>` : ''}
        <span class="tc-meta">${fmtTimeFull(tc.atMs)}</span>
        ${tc.durationMs != null ? `<span class="tc-meta">${fmtMs(tc.durationMs)}</span>` : ''}
        ${tc.failed ? '<span class="pill bad">failed</span>' : '<span class="pill ok">completed</span>'}
      </div>
      <div class="tc-args"><div class="dm-k">arguments</div>${args}</div>
      <div class="tc-args"><div class="dm-k">result</div>${out}
        ${tc.output != null ? `<div style="margin-top:6px">${copyBtn(tc.output, 'tool output')}</div>` : ''}</div>
    </div>`;
  }).join('');
  return `<div class="panel"><div class="panel-h"><div>
    <div class="panel-t">Tool execution</div>
    <div class="panel-sub">${tools.length} calls · actual arguments and outputs</div>
  </div></div>
    <div class="tc-list">${rows}</div>
  </div>`;
}

function errorsSection(content, t) {
  const errs = ((content && content.errors) || []).slice();
  if (t.errorMessage) errs.unshift({ atMs: t.endedAtMs, stage: 'trace', message: t.errorMessage });
  if (!errs.length) return '';
  const rows = errs.map(e => `
    <div class="event-row">
      <span class="event-ts">${e.atMs ? fmtTimeFull(e.atMs) : '—'}</span>
      <span class="event-name" style="color:var(--red)">${esc(e.stage || 'error')}</span>
      <span class="event-data">${esc(e.message || '')}</span>
    </div>`).join('');
  return `<div class="panel"><div class="panel-h"><div>
    <div class="panel-t">Errors</div>
    <div class="panel-sub">${errs.length} recorded failure${errs.length === 1 ? '' : 's'}</div>
  </div></div>
    <div class="events-list">${rows}</div>
  </div>`;
}

function rawEventsSection(events) {
  if (!events.length) return '';
  const rows = events.map(ev => {
    let pretty = '';
    if (ev.dataJson) {
      try { pretty = esc(JSON.stringify(JSON.parse(ev.dataJson), null, 1)); } catch { pretty = esc(ev.dataJson); }
    }
    return `<details class="raw-ev">
      <summary>
        <span class="event-ts">${fmtTimeFull(ev.atMs)}</span>
        <span class="event-name">${esc(ev.name)}</span>
        <span class="event-data">#${ev.seq}</span>
      </summary>
      ${pretty ? `<pre class="tc-pre" style="margin:8px 0 4px">${pretty}</pre>` : '<div class="faint" style="padding:6px 0">no payload</div>'}
    </details>`;
  }).join('');
  return `<div class="panel"><div class="panel-h"><div>
    <div class="panel-t">Raw event inspector</div>
    <div class="panel-sub">${events.length} structured events · expand to inspect payloads</div>
  </div></div>
    <div class="events-list">${rows}</div>
  </div>`;
}

/* Wire per-trace interactions: copy buttons and conversation search. */
function bindTraceDetailActions(content) {
  document.querySelectorAll('.copy-inline').forEach(b => {
    b.onclick = () => navigator.clipboard.writeText(b.dataset.copy || '').then(() => toast('copied'));
  });
  const search = $('#convSearch');
  if (search) {
    search.oninput = debounce(() => {
      const q = search.value.trim().toLowerCase();
      document.querySelectorAll('#convList .tc').forEach(node => {
        const text = node.textContent.toLowerCase();
        node.style.display = !q || text.includes(q) ? '' : 'none';
      });
    }, 150);
  }
}

function renderWaterfall(spans, trace) {
  const wf = $('#waterfall'); if (!wf) return;
  if (!spans.length) {
    wf.innerHTML = '<div class="empty" style="padding:30px"><div class="empty-s">No spans recorded</div></div>';
    return;
  }
  const t0 = Math.min(...spans.map(s => s.startMs));
  const t1 = Math.max(...spans.map(s => (s.endMs ?? s.startMs)));
  const span = Math.max(t1 - t0, 1);

  // Axis ticks: 0, 25, 50, 75, 100% of the trace window.
  let axis = '<div class="wf-axis">';
  for (let i = 0; i <= 4; i++) {
    const pct = i * 25;
    axis += `<span class="wf-tick" style="left:${pct}%">${fmtMs(Math.round(span * pct / 100))}</span>`;
  }
  axis += '</div>';

  let rows = '';
  for (const s of spans.sort((a, b) => a.startMs - b.startMs)) {
    const left = ((s.startMs - t0) / span) * 100;
    const dur = (s.endMs ?? s.startMs) - s.startMs;
    const width = Math.max((dur / span) * 100, 0.4);
    const cls = s.status === 'error' ? 'err'
      : (s.status === 'cancelled_user' || s.status === 'cancelled_client') ? 'cancel'
      : s.parentSpanId ? 'child' : '';
    rows += `<div class="wf-row" title="${esc(s.name)}">
      <span class="wf-label">${esc(s.name)}</span>
      <div class="wf-track">
        <div class="wf-bar ${cls}" style="left:${left}%;width:${width}%"></div>
        <span class="wf-time" style="left:calc(${Math.min(left + width, 88)}% + 8px)">${fmtMs(dur)}</span>
      </div></div>`;
  }
  wf.innerHTML = axis + rows;
}

function renderSpanTree(spans, trace) {
  const tree = $('#spanTree'); if (!tree) return;
  if (!spans.length) { tree.innerHTML = '<div class="empty" style="padding:24px"><div class="empty-s">No spans</div></div>'; return; }
  const byParent = new Map();
  for (const s of spans) {
    const key = s.parentSpanId || '__root__';
    if (!byParent.has(key)) byParent.set(key, []);
    byParent.get(key).push(s);
  }
  const node = (span, depth) => {
    const children = byParent.get(span.spanId) || [];
    const hasKids = children.length > 0;
    const wrap = el('div', 'span-node');
    const row = el('div', 'span-row');
    row.innerHTML = `<span class="span-caret">${hasKids ? '▾' : '·'}</span>
      <span class="span-name">${esc(span.name)}</span>
      <span class="span-kind">${esc(span.kind)}</span>
      ${statusPill(span.status)}
      <span class="span-dur">${fmtMs(span.durationMs)}</span>`;
    wrap.appendChild(row);
    if (hasKids) {
      const kids = el('div', 'span-children');
      for (const c of children.sort((a, b) => a.startMs - b.startMs)) kids.appendChild(node(c, depth + 1));
      wrap.appendChild(kids);
      row.onclick = () => kids.classList.toggle('hidden');
    } else {
      row.onclick = () => {};
    }
    return wrap;
  };
  const roots = byParent.get('__root__') || spans.filter(s => !s.parentSpanId);
  for (const r of roots.sort((a, b) => a.startMs - b.startMs)) tree.appendChild(node(r, 0));
}

function renderEvents(events) {
  const list = $('#eventsList'); if (!list) return;
  if (!events.length) {
    list.innerHTML = '<div class="empty" style="padding:24px"><div class="empty-s">No events recorded</div></div>';
    return;
  }
  for (const ev of events) {
    let data = '';
    if (ev.dataJson) {
      try { data = esc(JSON.stringify(JSON.parse(ev.dataJson))); } catch { data = esc(ev.dataJson); }
    }
    list.appendChild(el('div', 'event-row',
      `<span class="event-ts">${fmtTimeFull(ev.atMs)}</span>
       <span class="event-name">${esc(ev.name)}</span>
       <span class="event-data">${data}</span>`));
  }
}

/* ============================================================
   REQUESTS
   ============================================================ */
async function renderRequests() {
  $('#pageTitle').textContent = 'Requests';
  $('#pageSub').textContent = 'inference traffic, one row per request';
  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <div class="filters">
      <span class="filter-label">search</span>
      <div class="search-box"><input id="reqQ" placeholder="request id, trace id, model…" value="${esc(S.requests.q)}"></div>
      <span class="filter-label">status</span>
      <div class="seg" id="reqStatus">${['all','success','error','cancelled_user','cancelled_client','interrupted']
        .map(s => `<button data-s="${s}" class="${S.requests.status===s?'on':''}">${s.replace(/_/g,' ')}</button>`).join('')}</div>
      <button class="adv-toggle" id="reqRefresh">↻ refresh</button>
    </div>
    <div class="panel">
      <div class="table-scroll"><table>
        <thead><tr>
          <th>request</th><th>trace</th><th>model</th><th>status</th>
          <th>started</th><th>duration</th><th>tokens</th><th>ttft</th><th>cancel</th>
        </tr></thead>
        <tbody id="reqBody"></tbody>
      </table></div>
      <div class="tbl-foot">
        <span class="tbl-count" id="reqCount"></span>
        <div class="pager" id="reqPager"></div>
      </div>
    </div>
  </div></div>`;

  $('#reqQ').oninput = debounce(async e => {
    S.requests.q = e.target.value; S.requests.page = 1; await loadRequests();
  }, 250);
  $('#reqStatus').querySelectorAll('button').forEach(b => b.onclick = async () => {
    S.requests.status = b.dataset.s; S.requests.page = 1;
    $('#reqStatus').querySelectorAll('button').forEach(x => x.classList.toggle('on', x === b));
    await loadRequests();
  });
  $('#reqRefresh').onclick = () => loadRequests();
  await loadRequests();
}

async function loadRequests() {
  const params = new URLSearchParams({
    page: S.requests.page, per_page: S.requests.perPage,
  });
  if (S.requests.q) params.set('q', S.requests.q);
  if (S.requests.status !== 'all') params.set('status', S.requests.status);
  let body;
  try { body = await api('/api/requests?' + params); }
  catch (e) { $('#reqBody').innerHTML = `<tr><td colspan="9" style="color:var(--red)">${esc(e.message)}</td></tr>`; return; }
  Object.assign(S.requests, { items: body.items, total: body.total, pages: body.pages });
  const tb = $('#reqBody');
  tb.innerHTML = '';
  if (!body.items.length) {
    tb.innerHTML = `<tr><td colspan="9"><div class="empty"><div class="empty-t">No requests</div></div></td></tr>`;
  }
  for (const r of body.items) {
    const tr = el('tr');
    tr.innerHTML = `
      <td class="mono dim" title="${esc(r.requestId)}">${esc(String(r.requestId).slice(0, 8))}…</td>
      <td class="mono dim" title="open trace"><a href="#traces" class="trace-link" data-id="${esc(r.traceId)}" style="color:var(--violet)">${esc(String(r.traceId).slice(0, 8))}…</a></td>
      <td class="mono dim">${esc(r.model || '—')}</td>
      <td>${statusPill(r.status)}</td>
      <td class="mono faint">${fmtTime(r.startedAtMs)}</td>
      <td class="mono dim">${fmtMs(r.durationMs)}</td>
      <td class="mono dim">${r.inputTokens != null ? fmtInt(r.inputTokens) + '→' + fmtInt(r.outputTokens ?? 0) : '—'}</td>
      <td class="mono dim">${fmtMs(r.ttftMs)}</td>
      <td class="mono faint">${esc(r.cancelReason || '—')}</td>`;
    tb.appendChild(tr);
  }
  tb.querySelectorAll('.trace-link').forEach(a => a.onclick = e => {
    e.preventDefault(); e.stopPropagation();
    openTraceDetail(a.dataset.id);
  });
  $('#reqCount').textContent = `showing ${(body.page-1)*body.perPage+1}–${Math.min(body.page*body.perPage, body.total)} of ${fmtInt(body.total)}`;
  renderPager($('#reqPager'), body, p => { S.requests.page = p; loadRequests(); });
}

/* ============================================================
   MODELS
   ============================================================ */
async function renderModels() {
  $('#pageTitle').textContent = 'Models';
  $('#pageSub').textContent = 'models the runtime genuinely supports';
  $('#viewRoot').innerHTML = '<div class="content"><div class="content-inner"><div class="panel skeleton" style="height:300px"></div></div></div>';
  let models, params;
  try {
    [models, params] = await Promise.all([api('/api/models'), api('/api/models/params')]);
  } catch (e) { return errorView(e); }

  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Available models</div>
      <div class="panel-sub">${models.models.length} configured · providers: ${esc((models.configuredProviders || []).join(', ') || 'none')}</div>
    </div></div>
      <div class="table-scroll"><table>
        <thead><tr><th>model</th><th>provider</th><th>api</th><th>context</th><th>max output</th><th>reasoning</th><th>modalities</th></tr></thead>
        <tbody>${models.models.length ? models.models.map(m => `
          <tr style="cursor:default">
            <td class="mono dim">${esc(m.name)}</td>
            <td class="mono dim">${esc(m.provider)}</td>
            <td class="mono faint">${esc(m.api)}</td>
            <td class="mono dim">${fmtInt(m.contextWindow)}</td>
            <td class="mono dim">${fmtInt(m.maxTokens)}</td>
            <td>${m.reasoning ? '<span class="pill ok">yes</span>' : '<span class="pill inc">no</span>'}</td>
            <td class="mono faint">${(m.inputModalities || []).join(', ')}</td>
          </tr>`).join('') : `<tr><td colspan="7"><div class="empty"><div class="empty-t">No models configured</div>
            <div class="empty-s">Run <b>loop</b> and use /login, or set an API key env var.</div></div></td></tr>`}
        </tbody></table></div>
    </div>
    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Supported inference parameters</div>
      <div class="panel-sub">what the engine actually sends — nothing else is exposed</div>
    </div></div>
      <div style="padding:15px 18px;font-family:var(--mono);font-size:11.5px;line-height:2">
        <div><span class="pill ok">supported</span> ${(params.supported || []).join(' · ')}</div>
        <div><span class="pill bad">unsupported</span> ${(params.unsupported || []).join(' · ')}
          <span class="faint">— the engine never sends these, so no controls exist for them</span></div>
      </div>
    </div>
  </div></div>`;
}

/* ============================================================
   ANALYTICS
   ============================================================ */
async function renderAnalytics() {
  $('#pageTitle').textContent = 'Analytics';
  $('#pageSub').textContent = 'computed from persisted traces';
  $('#viewRoot').innerHTML = '<div class="content"><div class="content-inner">' +
    '<div class="kpis">' + '<div class="kpi skeleton" style="height:104px"></div>'.repeat(6) + '</div></div></div>';
  let a;
  try { a = await api('/api/analytics'); }
  catch (e) { return errorView(e); }

  const byStatus = a.byStatus || {};
  const total = a.totalRequests || 0;
  const k = a.latency || {}, tt = a.ttft || {};
  const tok = a.tokens || {};

  const kpis = [
    ['total requests', fmtInt(total), 'all statuses', 'var(--blue)'],
    ['successful', fmtInt(byStatus.success || 0), total ? (100*(byStatus.success||0)/total).toFixed(1) + '%' : '—', 'var(--acc)'],
    ['failed', fmtInt(byStatus.error || 0), total ? (100*(byStatus.error||0)/total).toFixed(1) + '%' : '—', 'var(--red)'],
    ['cancelled', fmtInt((byStatus.cancelled_user||0)+(byStatus.cancelled_client||0)), 'user + client disconnects', 'var(--amber)'],
    ['interrupted', fmtInt(byStatus.interrupted || 0), 'process exited mid-flight', 'var(--ink-faint)'],
    ['requests/sec', a.requestsPerSecond != null ? a.requestsPerSecond.toFixed(3) : '—', 'over the observed span', 'var(--violet)'],
  ];

  const statRow = (name, s) => `
    <tr style="cursor:default"><td class="mono dim">${name}</td>
      <td class="mono">${fmtInt(s.count || 0)}</td>
      <td class="mono dim">${fmtMs(s.p50Ms)}</td>
      <td class="mono dim">${fmtMs(s.p95Ms)}</td>
      <td class="mono dim">${fmtMs(s.p99Ms)}</td>
      <td class="mono dim">${s.meanMs != null ? s.meanMs.toFixed(1) + ' ms' : '—'}</td></tr>`;

  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <div class="kpis">${kpis.map(x =>
      `<div class="kpi" style="--kcol:${x[3]}"><div class="kpi-k">${x[0]}</div>
       <div class="kpi-v">${x[1]}</div><div class="kpi-s">${x[2]}</div></div>`).join('')}
    </div>
    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Latency distribution</div>
      <div class="panel-sub">nearest-rank percentiles over completed traces — never derived from averages</div>
    </div></div>
      <div class="table-scroll"><table>
        <thead><tr><th>metric</th><th>samples</th><th>p50</th><th>p95</th><th>p99</th><th>mean</th></tr></thead>
        <tbody>
          ${statRow('total duration', k)}
          ${statRow('time to first token', tt)}
        </tbody></table></div>
    </div>
    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Token throughput</div>
      <div class="panel-sub">sums over the observed window</div>
    </div></div>
      <div style="padding:15px 18px;font-family:var(--mono);font-size:12px" class="dim">
        input ${fmtInt(tok.input || 0)} tokens · output ${fmtInt(tok.output || 0)} tokens
      </div>
    </div>
    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Per-model usage</div>
      <div class="panel-sub">from persisted traces only</div>
    </div></div>
      <div class="table-scroll"><table>
        <thead><tr><th>model</th><th>requests</th><th>tokens in</th><th>tokens out</th></tr></thead>
        <tbody>${(a.models || []).map(m => `
          <tr style="cursor:default"><td class="mono dim">${esc(m.model)}</td>
            <td class="mono dim">${fmtInt(m.requests)}</td>
            <td class="mono dim">${fmtInt(m.inputTokens)}</td>
            <td class="mono dim">${fmtInt(m.outputTokens)}</td></tr>`).join('')
          || '<tr><td colspan="4" class="faint">no data</td></tr>'}
        </tbody></table></div>
    </div>
  </div></div>`;
}

/* ============================================================
   SETTINGS
   ============================================================ */
async function renderSettings() {
  $('#pageTitle').textContent = 'Settings';
  $('#pageSub').textContent = 'dashboard-owned configuration';
  $('#viewRoot').innerHTML = '<div class="content"><div class="content-inner"><div class="panel skeleton" style="height:220px"></div></div></div>';
  let s, health;
  try { [s, health] = await Promise.all([api('/api/settings'), api('/api/health')]); }
  catch (e) { return errorView(e); }

  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Trace content capture</div>
      <div class="panel-sub">message text inside traces — off by default</div>
    </div></div>
      <div style="padding:16px 18px">
        <label style="display:flex;gap:10px;align-items:center;cursor:pointer">
          <input type="checkbox" id="captureChk" ${s.captureContent ? 'checked' : ''}>
          <span>Capture prompt/response content into trace records</span>
        </label>
        <div class="param-hint" style="margin-top:8px;max-width:560px">
          Conversations (the playground) always persist their messages — that is their purpose and they are deletable.
          This switch governs additional content copies inside trace records. Traces already store operational
          metadata: model, parameters, timings, token counts and status. No credentials are ever recorded.
        </div>
      </div>
    </div>
    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Retention</div>
      <div class="panel-sub">oldest traces are deleted beyond this count</div>
    </div></div>
      <div style="padding:16px 18px;display:flex;gap:12px;align-items:center">
        <span class="mono dim" style="font-size:12px">keep at most</span>
        <input id="retentionInput" type="number" min="100" max="10000000" value="${s.retentionMaxTraces}"
          style="background:var(--bg);border:1px solid var(--line2);color:var(--ink);border-radius:8px;font-family:var(--mono);font-size:12px;padding:7px 11px;width:130px">
        <span class="mono dim" style="font-size:12px">traces</span>
        <button class="new-chat-btn" id="saveSettings">save</button>
      </div>
    </div>
    <div class="panel"><div class="panel-h"><div>
      <div class="panel-t">Storage</div>
      <div class="panel-sub">durable backend state</div>
    </div></div>
      <div style="padding:16px 18px;font-family:var(--mono);font-size:11.5px;line-height:2" class="dim">
        <div>persisted traces: <b>${fmtInt(health.consoleTraces ?? 0)}</b></div>
        <div>persisted requests: <b>${fmtInt(health.consoleRequests ?? 0)}</b></div>
        <div>storage: SQLite (WAL, synchronous=FULL)</div>
        <div>recovery: rows left running by a crash are marked <span class="pill inc">interrupted</span>, never converted to success</div>
      </div>
    </div>
  </div></div>`;

  $('#saveSettings').onclick = async () => {
    const max = parseInt($('#retentionInput').value, 10);
    if (isNaN(max) || max < 100) { toast('retention must be ≥ 100'); return; }
    try {
      await api('/api/settings', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          captureContent: $('#captureChk').checked,
          retentionMaxTraces: max,
        }),
      });
      toast('settings saved');
    } catch (e) { toast(e.message); }
  };
}

/* ============================================================
   FUN TIME — GAMES WORKSPACE (route: funtime)
   Phase 10 created this destination with exactly two entries; Phase 11
   made both playable. This revision adds an arcade-style selection
   screen (#funtime) with two large cards, and gives each game its own
   dedicated full-width view (#funtime/t3, #funtime/si) so neither is
   ever cramped beside the other. The Dino Runner stays on the sign-in
   screen only and is deliberately not offered here.
   All game state lives in page memory; nothing touches any database.
   ============================================================ */

/* ---- Tic-Tac-Toe: pure game logic --------------------------------
 * Board encoding: a 9-char array of 'X' (human), 'O' (AI) or '' (empty).
 * Everything below is a pure function over that array so the rules can be
 * exhaustively verified without a DOM. The UI layer owns no rules. */
const T3 = (() => {
  const LINES = [
    [0, 1, 2], [3, 4, 5], [6, 7, 8],   // rows
    [0, 3, 6], [1, 4, 7], [2, 5, 8],   // columns
    [0, 4, 8], [2, 4, 6],              // diagonals
  ];

  /** Winner and the winning line, or null if undecided. */
  function outcome(b) {
    for (const L of LINES) {
      const v = b[L[0]];
      if (v && v === b[L[1]] && v === b[L[2]]) return { winner: v, line: L };
    }
    return null;
  }

  /** True when the board is full (and, with no winner, therefore a draw). */
  const full = b => b.every(c => c !== '');

  /** Terminal result: {kind:'win'|'loss'|'draw'} from the human's view,
   *  or null while the game is still in progress. */
  function result(b) {
    const o = outcome(b);
    if (o) return o.winner === 'X'
      ? { kind: 'win', line: o.line } : { kind: 'loss', line: o.line };
    if (full(b)) return { kind: 'draw', line: null };
    return null;
  }

  /** Indices still open. */
  const moves = b => b.map((c, i) => c === '' ? i : -1).filter(i => i >= 0);

  /**
   * Minimax for O (the maximiser). Returns the score from O's perspective:
   * +10 - depth for an O win, depth - 10 for an X win, 0 for a draw.
   * Depth is folded into the score so the AI prefers the quickest win and
   * the slowest loss — it never throws away a faster mate.
   *
   * `b` is treated as read-only: hypothetical moves are made on a copy and
   * undone is unnecessary because the copy is discarded, so the caller's
   * live board can never be mutated by the search.
   */
  function minimax(b, depth, isOTurn) {
    const r = result(b);
    if (r) {
      if (r.kind === 'win') return depth - 10;   // X won: worse later
      if (r.kind === 'loss') return 10 - depth;  // O won: better sooner
      return 0;                                   // draw
    }
    let best = isOTurn ? -Infinity : Infinity;
    for (const m of moves(b)) {
      const next = b.slice();
      next[m] = isOTurn ? 'O' : 'X';
      const s = minimax(next, depth + 1, !isOTurn);
      best = isOTurn ? Math.max(best, s) : Math.min(best, s);
    }
    return best;
  }

  /**
   * The AI's chosen move for a board and difficulty.
   * Returns an index in 0..8 that is always currently empty, or null when
   * the board is terminal / has no move to make.
   *
   * easy   — uniformly random legal move (never an occupied cell).
   * medium — take an immediate win, else block an immediate X win, else a
   *          preference-ordered sensible move (centre, corner, edge).
   * impossible — full minimax; deterministic (first best-scoring move).
   */
  function pick(board, difficulty) {
    if (result(board)) return null;                 // game already over
    const open = moves(board);
    if (!open.length) return null;

    if (difficulty === 'easy') {
      return open[Math.floor(Math.random() * open.length)];
    }

    if (difficulty === 'medium') {
      // 1) Win immediately if possible.
      for (const m of open) {
        const t = board.slice(); t[m] = 'O';
        if (outcome(t)?.winner === 'O') return m;
      }
      // 2) Block an immediate human win.
      for (const m of open) {
        const t = board.slice(); t[m] = 'X';
        if (outcome(t)?.winner === 'X') return m;
      }
      // 3) Sensible heuristic order: centre, corners, edges.
      const pref = [4, 0, 2, 6, 8, 1, 3, 5, 7];
      for (const m of pref) if (board[m] === '') return m;
      return open[0];
    }

    // impossible: exhaustive minimax. Deterministic — ties resolve to the
    // first move in index order, so the same board always yields the same
    // choice and the behaviour is reproducible in tests.
    let bestScore = -Infinity, bestMove = null;
    for (const m of open) {
      const next = board.slice();
      next[m] = 'O';
      const s = minimax(next, 1, false);
      if (s > bestScore) { bestScore = s; bestMove = m; }
    }
    return bestMove;
  }

  return { LINES, outcome, result, moves, minimax, pick };
})();

/* ---- Space Invaders: pure game logic -------------------------------
 * Phase 11. All rules live in SI (no DOM, no timers) so they can be driven
 * deterministically in tests. The UI layer owns rendering and timing only.
 * Arcade Phase 1: automatic level progression (LEVEL COMPLETE transition,
 * next level starts on its own) + progressive, capped difficulty.
 * Arcade Phase 2: automatic power-up rewards — one ability granted after
 * every completed level, fair distribution, no stacking, correct expiry.
 *
 * Coordinate system: logical units, origin top-left, y grows downward.
 * W is the playfield width; the height is implicit in the layout below.
 * Collisions are computed purely in logical units, never device pixels. */
const SI = (() => {
  /* Fixed logical playfield. The canvas scales this to whatever size it
   * is displayed at; nothing in the rules ever sees a pixel. */
  const W = 480, H = 620;
  const PLAYER_Y = H - 46;          // player's fixed vertical line
  const DANGER_Y = H - 96;          // enemies below this end the game
  const PLAYER_W = 34, PLAYER_H = 18;
  const BULLET_W = 3, BULLET_H = 12;
  const BOMB_W = 4, BOMB_H = 10;
  /* Auto-fire cadence: one shot every 0.45 s of simulated time. Chosen so
   * the game is playable with steering alone; a full wave stays winnable
   * because the cooldown is time-based, not frame-based. */
  const SI_FIRE_EVERY = 0.45;

  /* Rows, top to bottom. Row 0 scores highest (classic arcade scoring). */
  const ROWS = [
    { type: 'a', pts: 30, color: '#ff8fab' },
    { type: 'b', pts: 20, color: '#ffd7ba' },
    { type: 'c', pts: 20, color: '#c9f2ff' },
    { type: 'd', pts: 10, color: '#9bf2c0' },
    { type: 'e', pts: 10, color: '#9bf2c0' },
  ];
  const COLS = 10;
  const CELL_W = 36, CELL_H = 32;

  /** Build the formation for a wave: COLS x ROWS grid of live enemies.
   *  Phase 1 progressive difficulty: from level 3 the formation gains a
   *  6th row (drones) and from level 5 an extra column, both capped so
   *  the grid always fits the field and stays winnable. */
  function waveRows(wave) { return Math.min(ROWS.length + (wave >= 3 ? 1 : 0), 6); }
  function waveCols(wave) { return Math.min(COLS + (wave >= 5 ? 1 : 0), 11); }
  function makeWave(wave) {
    const enemies = [];
    const rows = waveRows(wave), cols = waveCols(wave);
    const startX = (W - (cols - 1) * CELL_W) / 2;
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const rowDef = ROWS[Math.min(r, ROWS.length - 1)];
        enemies.push({
          id: r * cols + c,
          row: r, col: c,
          x: startX + c * CELL_W,
          y: 70 + r * CELL_H,
          w: 24, h: 16,
          alive: true,
          pts: rowDef.pts,
          color: rowDef.color,
          type: rowDef.type,
        });
      }
    }
    return enemies;
  }

  /* ---- Phase 1 difficulty scaling -----------------------------------
   * Every knob is a smooth function of the level with a hard cap, so
   * difficulty rises gradually and never becomes unplayable:
   *  - speed:       26 -> 64 px/s over 9 levels (+4.5/level)
   *  - descend:     10 -> 20 px/reversal over 8 levels (+1.25/level)
   *  - bomb rate:   0.55 -> 1.15 bombs/s over 6 levels (+0.1/level)
   *  - bomb speed:  190 -> 274 px/s over ~7 levels (+12/level)
   * The engine stays pure: no DOM, no timers, fully testable. */
  const DIFF = {
    SPEED_BASE: 26,  SPEED_STEP: 4.5,  SPEED_MAX: 64,
    DESC_BASE: 10,   DESC_STEP: 1.25,  DESC_MAX: 20,
    RATE_BASE: 0.55, RATE_STEP: 0.10,  RATE_MAX: 1.15,
    BSPEED_BASE: 190, BSPEED_STEP: 12, BSPEED_MAX: 274,
  };
  function clampStep(base, step, max, wave) {
    return Math.min(base + (wave - 1) * step, max);
  }
  function waveSpeed(wave)  { return clampStep(DIFF.SPEED_BASE,  DIFF.SPEED_STEP,  DIFF.SPEED_MAX,  wave); }
  function waveDescend(wave){ return clampStep(DIFF.DESC_BASE,   DIFF.DESC_STEP,   DIFF.DESC_MAX,   wave); }
  function bombRate(wave)   { return clampStep(DIFF.RATE_BASE,   DIFF.RATE_STEP,   DIFF.RATE_MAX,   wave); }
  function bombSpeed(wave)  { return clampStep(DIFF.BSPEED_BASE, DIFF.BSPEED_STEP, DIFF.BSPEED_MAX, wave); }

  /* ---- Phase 2: power-up rewards -------------------------------------
   * One ability is granted automatically after every completed level.
   * All rules live here in the pure engine so they are testable:
   *
   *  - TEMPORARY abilities carry a duration (seconds of simulated time)
   *    and expire inside step(); they tick only while combat is being
   *    simulated, so pausing freezes them exactly like everything else.
   *  - EXTRA LIFE is permanent for the run (never expires), capped at
   *    MAX_LIVES so it cannot be farmed into infinity.
   *  - Re-awarding an already-active temporary ability REFRESHES its
   *    duration instead of stacking (no unlimited stacking).
   *  - Distribution is fair: a fixed rotation covers all six before any
   *    repeats, then reshuffles — no ability is starved or spammed.
   *  - Durations scale mildly with the wave number so power-ups stay
   *    useful as enemy speed/fire caps rise. */
  const MAX_LIVES = 5;
  const POWERS = {
    rapid:    { label: 'RAPID FIRE',   temp: true,  dur: 8,  growth: 0.6 },
    triple:   { label: 'TRIPLE SHOT',  temp: true,  dur: 7,  growth: 0.5 },
    shield:   { label: 'SHIELD',       temp: true,  dur: 10, growth: 0.8 },
    double:   { label: 'DOUBLE DAMAGE',temp: true,  dur: 9,  growth: 0.7 },
    life:     { label: 'EXTRA LIFE',   temp: false },
    slow:     { label: 'SLOW MOTION',  temp: true,  dur: 8,  growth: 0.6 },
  };
  /* Fixed rotation: all six appear once each before anything repeats.
   * Ordered so early levels get broadly useful, easy-to-read powers. */
  const POWER_ROTATION = ['rapid', 'triple', 'shield', 'double', 'life', 'slow'];

  /** Duration for a temporary power at a given wave (seconds). */
  function powerDuration(kind, wave) {
    const p = POWERS[kind];
    if (!p || !p.temp) return 0;
    return p.dur + Math.max(0, wave - 1) * p.growth;
  }

  /** Which power-up the NEXT completed level awards. Pure: derived from
   *  the rotation index, which lives in the game state. */
  function nextPower(g) {
    return POWER_ROTATION[g.powerIdx % POWER_ROTATION.length];
  }

  /** Grant a power-up to g. Returns the kind granted. Refreshes (rather
   *  than stacks) temporaries; EXTRA LIFE respects MAX_LIVES. */
  function grantPower(g, kind) {
    const p = POWERS[kind];
    if (!p) return null;
    g.lastPower = kind;
    if (!p.temp) {                                  // EXTRA LIFE
      if (g.lives < MAX_LIVES) g.lives++;
      g.lifeCapped = g.lives >= MAX_LIVES;          // UI hint when full
      return kind;
    }
    const t = g.powers[kind];
    if (t && t.active) {                            // refresh, don't stack
      t.left = powerDuration(kind, g.wave);
      t.refreshed = true;
    } else {
      g.powers[kind] = { active: true, left: powerDuration(kind, g.wave), total: powerDuration(kind, g.wave) };
    }
    return kind;
  }

  /** Advance the rotation and grant the next reward. Called by the engine
   *  when a level completes (inside startTransition). */
  function awardNextPower(g) {
    const kind = nextPower(g);
    grantPower(g, kind);
    g.powerIdx = (g.powerIdx || 0) + 1;
    return kind;
  }

  /** Tick every temporary power for dt seconds; collect expiries so the
   *  UI can animate them. Pure engine, called from step(). */
  function tickPowers(g, dt) {
    const expired = [];
    for (const kind of Object.keys(g.powers)) {
      const t = g.powers[kind];
      if (!t || !t.active) continue;
      t.left -= dt;
      if (t.left <= 0) {
        t.active = false; t.left = 0;
        expired.push(kind);
      }
    }
    return expired;
  }

  /** A fresh game state object. Pure data; no timers, no DOM. */
  function newGame() {
    return {
      state: 'ready',                // ready | playing | paused | over
      score: 0,
      highScore: 0,
      lives: 3,
      wave: 1,
      player: { x: W / 2 - PLAYER_W / 2, y: PLAYER_Y, w: PLAYER_W, h: PLAYER_H },
      enemies: makeWave(1),
      bullets: [],                   // player shots (upward)
      bombs: [],                     // enemy shots (downward)
      fx: [],                        // explosions, bounded
      dir: 1,                        // formation direction (+1/-1)
      speed: waveSpeed(1),
      descend: waveDescend(1),
      bombTimer: 0,
      bombEvery: 1 / bombRate(1),
      bombVy: bombSpeed(1),          // Phase 1: enemy projectile speed/level
      elapsed: 0,
      dangerY: DANGER_Y,
      /* Phase 1 automatic progression. `transition` is a pure countdown in
       * seconds while state === 'between': LEVEL COMPLETE has been shown,
       * the next level starts by itself when it reaches 0 — no button. */
      transition: 0,
      /* Phase 2 power-up state. `powers[kind] = {active, left, total}` for
       * temporaries; `powerIdx` drives the fair rotation; `lastPower` and
       * `justGotPower` are one-frame signals consumed by the UI layer. */
      powers: {},
      powerIdx: 0,
      lastPower: null,
      justGotPower: null,
      lifeCapped: false,
    };
  }

  /* AABB overlap in logical units. */
  function hit(a, b) {
    return a.x < b.x + b.w && a.x + a.w > b.x &&
           a.y < b.y + b.h && a.y + a.h > b.y;
  }

  /** Spawn the next wave: keep score/lives, raise difficulty.
   *  Phase 1: called by the automatic transition (or tests) only — never
   *  directly from step(), which now routes through startTransition.
   *  Phase 2: active power-ups are deliberately NOT cleared here; valid
   *  upgrades carry across the level boundary. */
  function nextWave(g) {
    g.wave++;
    g.enemies = makeWave(g.wave);
    g.bullets = []; g.bombs = []; g.fx = [];
    g.dir = 1;
    g.speed = waveSpeed(g.wave);
    g.descend = waveDescend(g.wave);
    g.bombTimer = 0;
    g.bombEvery = 1 / bombRate(g.wave);
    g.bombVy = bombSpeed(g.wave);   // enemy projectiles fly faster each level
  }

  /* Phase 1: how long the LEVEL COMPLETE beat lasts before the next
   * level starts automatically (seconds of simulated time). */
  const TRANSITION_SECS = 1.6;

  /** Enter the between-levels beat: keep everything on screen, stop
   *  simulating combat, and let the UI show LEVEL COMPLETE. The next
   *  level begins on its own when the countdown finishes.
   *  Phase 2: this is also the reward moment — the power-up is granted
   *  here, immediately, so it is already active when the next level
   *  starts (and survives the transition by construction). */
  function startTransition(g) {
    g.state = 'between';
    g.transition = TRANSITION_SECS;
    g.bombs = [];                    // clear incoming fire for a fair break
    g.cool = 0;                      // fresh auto-fire cadence next level
    g.justGotPower = awardNextPower(g);   // Phase 2: automatic reward
  }

  /** One deterministic simulation step. `input` = {left,right,fire}.
   *  `dt` is seconds. The caller normally clamps, but the engine defends
   *  itself as well: a huge dt (tab suspension, debugger pause) can never
   *  teleport entities across the field or skip collisions. */
  function step(g, input, dt) {
    if (g.state !== 'playing' && g.state !== 'between') return;
    if (!isFinite(dt) || dt <= 0) return;
    if (dt > 0.05) dt = 0.05;                 // hard ceiling: one frame
    g.elapsed += dt;

    /* --- Phase 1: automatic level progression ---
     * Between levels we freeze combat but keep ticking simulated time so
     * the countdown is pause-safe and tab-suspension-proof. When it hits
     * zero the next level starts by itself — no player input required. */
    if (g.state === 'between') {
      g.transition -= dt;
      // Explosions keep fading during the beat; nothing else moves.
      g.fx = g.fx.filter(f => (f.t += dt) < f.life);
      if (g.transition <= 0) {
        nextWave(g);
        g.state = 'playing';
        g.justLeveled = true;                 // signal for the UI banner
      }
      return;
    }

    /* --- Phase 2: temporary power-ups tick only while combat runs ---
     * They are frozen by pause and by the between-levels beat exactly
     * like every other timer in the engine. */
    const expired = tickPowers(g, dt);
    if (expired.length) g.powerExpired = expired;   // one-frame UI signal

    /* --- player --- */
    const PSPEED = 240;
    let dx = 0;
    if (input.left) dx -= 1;
    if (input.right) dx += 1;
    g.player.x += dx * PSPEED * dt;
    // Clamp inside the field; never negative, never past the right edge.
    if (g.player.x < 0) g.player.x = 0;
    if (g.player.x + g.player.w > W) g.player.x = W - g.player.w;

    /* Active-power lookups used below (all default to safe values). */
    const pw = k => !!(g.powers[k] && g.powers[k].active);

    /* --- player bullet: AUTOMATIC firing ---
     * The ship fires itself while playing; the player only steers. Cadence
     * is driven by the existing cooldown accumulator, so it is tied to
     * simulated time (dt), never to frame count or a separate timer.
     * Space/FIRE remain available as an optional manual trigger, but the
     * game never requires them.
     * Phase 2: RAPID FIRE halves the cooldown; TRIPLE SHOT adds two
     * angled shots; DOUBLE DAMAGE marks bullets to pierce one extra hit. */
    const fireEvery = pw('rapid') ? SI_FIRE_EVERY / 2 : SI_FIRE_EVERY;
    const wantFire = true;                   // auto-fire is always on
    if ((wantFire || input.fire) && !g.cool) {
      const shots = [];
      const cx = g.player.x + g.player.w / 2 - BULLET_W / 2;
      shots.push({ x: cx, y: g.player.y - BULLET_H, w: BULLET_W, h: BULLET_H, vy: -420 });
      if (pw('triple')) {                    // two extra angled projectiles
        shots.push({ x: cx - 12, y: g.player.y - BULLET_H + 4, w: BULLET_W, h: BULLET_H, vy: -420, vx: -70 });
        shots.push({ x: cx + 12, y: g.player.y - BULLET_H + 4, w: BULLET_W, h: BULLET_H, vy: -420, vx: 70 });
      }
      for (const s of shots) {
        if (pw('double')) s.dmg = 2;         // DOUBLE DAMAGE pierces once
        g.bullets.push(s);
      }
      g.cool = fireEvery;                    // fire cooldown (seconds)
      g.fired = true;                        // signal for the UI's SFX hook
    }
    if (g.cool != null && g.cool > 0) g.cool -= dt;
    if (g.cool != null && g.cool < 0) g.cool = 0;

    /* --- formation movement --- */
    const alive = g.enemies.filter(e => e.alive);
    // Fewer enemies -> faster (classic behaviour), gently. Normalised
    // against THIS wave's full size so bigger later formations aren't
    // accidentally slower than their base speed (Phase 1).
    const fullCount = waveRows(g.wave) * waveCols(g.wave);
    const countBoost = 1 + (1 - alive.length / fullCount) * 0.6;
    // Phase 2: SLOW MOTION halves enemy movement while it lasts.
    const slowMul = pw('slow') ? 0.5 : 1;
    const spd = g.speed * countBoost * slowMul;
    let minX = Infinity, maxX = -Infinity;
    for (const e of alive) {
      if (e.x < minX) minX = e.x;
      if (e.x + e.w > maxX) maxX = e.x + e.w;
    }
    const nextMin = minX + spd * g.dir * dt;
    const nextMax = maxX + spd * g.dir * dt;
    if ((g.dir > 0 && nextMax >= W) || (g.dir < 0 && nextMin <= 0)) {
      // Reverse and descend.
      for (const e of alive) e.y += g.descend;
      g.dir = -g.dir;
    } else {
      for (const e of alive) e.x += spd * g.dir * dt;
    }

    /* --- enemy bombs ---
     * Phase 2: SLOW MOTION slows the firing cadence too (bombEvery is
     * stretched), matching the slower world the player sees. */
    g.bombTimer += dt;
    const bombEvery = pw('slow') ? g.bombEvery / 0.5 : g.bombEvery;
    if (g.bombTimer >= bombEvery && alive.length) {
      g.bombTimer = 0;
      // Fire from a random alive column's lowest enemy.
      const cols = {};
      for (const e of alive) {
        if (!cols[e.col] || e.y > cols[e.col].y) cols[e.col] = e;
      }
      const shooters = Object.values(cols);
      const s = shooters[Math.floor(Math.random() * shooters.length)];
      g.bombs.push({ x: s.x + s.w / 2 - BOMB_W / 2, y: s.y + s.h,
                     w: BOMB_W, h: BOMB_H, vy: g.bombVy });
    }

    /* --- move projectiles ---
     * Phase 2: SLOW MOTION slows the whole enemy world — formation AND
     * bombs already in flight — while the player moves at full speed. */
    for (const b of g.bullets) { b.y += b.vy * dt; if (b.vx) b.x += b.vx * dt; }
    for (const b of g.bombs) b.y += b.vy * slowMul * dt;

    /* --- collisions: bullet vs enemy ---
     * Phase 2: DOUBLE DAMAGE bullets carry dmg=2 and keep flying through
     * the first enemy they kill, so one shot can clear two. */
    for (const b of g.bullets) {
      if (b.dead) continue;
      let pierce = b.dmg === 2 ? 1 : 0;            // extra kills allowed
      for (const e of g.enemies) {
        if (!e.alive || e.dead) continue;
        if (hit(b, e)) {
          e.alive = false; e.dead = true;
          g.score += e.pts;                       // awarded exactly once
          if (g.score > g.highScore) g.highScore = g.score;
          g.fx.push({ x: e.x, y: e.y, t: 0, life: 0.35, pts: e.pts });
          if (pierce > 0) { pierce--; continue; } // pierce: keep going
          b.dead = true;
          break;
        }
      }
    }

    /* --- collisions: bomb vs player ---
     * Phase 2: SHIELD absorbs the hit instead of losing a life, but the
     * shield is consumed by doing so (one block per award). */
    for (const b of g.bombs) {
      if (b.dead) continue;
      if (hit(b, g.player)) {
        b.dead = true;
        if (pw('shield')) {
          g.powers.shield.left = 0;                // consumed on the block
          g.powers.shield.active = false;
          g.powerExpired = ['shield'];
          g.shieldBlocked = true;                  // Phase 4: SFX signal
          g.fx.push({ x: g.player.x, y: g.player.y, t: 0, life: 0.5, shieldBlock: true });
        } else {
          g.lives = Math.max(0, g.lives - 1);       // never negative
          g.fx.push({ x: g.player.x, y: g.player.y, t: 0, life: 0.5, hitPlayer: true });
          if (g.lives <= 0) { g.state = 'over'; }
        }
      }
    }

    /* --- danger line: enemies reached the player --- */
    for (const e of g.enemies) {
      if (e.alive && e.y + e.h >= g.dangerY) { g.state = 'over'; break; }
    }

    /* --- cleanup off-screen and dead entities --- */
    g.bullets = g.bullets.filter(b => !b.dead && b.y + b.h > 0 &&
                                      b.x > -20 && b.x < W + 20);
    g.bombs = g.bombs.filter(b => !b.dead && b.y < H);
    g.enemies = g.enemies.filter(e => e.alive);
    // Explosions expire by age; the list is naturally bounded by the
    // enemy count, but cap defensively anyway.
    g.fx = g.fx.filter(f => (f.t += dt) < f.life).slice(-60);

    /* --- wave cleared? ---
     * Phase 1: enter the LEVEL COMPLETE beat; the next level starts
     * automatically when it ends. Score, lives and high score carry over
     * untouched inside g. */
    if (!g.enemies.length && g.state === 'playing') {
      startTransition(g);
    }
  }

  return { W, H, PLAYER_Y, DANGER_Y, ROWS, COLS, CELL_W, CELL_H,
           FIRE_EVERY: SI_FIRE_EVERY, DIFF, TRANSITION_SECS,
           MAX_LIVES, POWERS, POWER_ROTATION,
           powerDuration, nextPower, grantPower, awardNextPower, tickPowers,
           makeWave, waveRows, waveCols, waveSpeed, waveDescend,
           bombRate, bombSpeed,
           newGame, nextWave, startTransition, step, hit };
})();

/* ---- Space Invaders: UI + lifecycle --------------------------------
 * Owns the canvas, the single rAF loop, listeners and teardown. Every
 * listener is registered through SIG.lifecycle so leaving the route
 * removes all of them; `epoch` makes stale callbacks inert. */
const SIG = {
  g: null,                  // current SI.newGame() state
  ctx: null,
  raf: 0,
  lastT: 0,
  epoch: 0,                 // bumped on every teardown/restart
  keys: Object.create(null),
  touch: { left: false, right: false, fire: false },
  cleanup: [],              // [fn, ...] run on teardown
  reducedMotion: false,
};

/* ---- Space Invaders: audio ------------------------------------------
 * Phase 3 + Phase 4: a full synthesised music and sound-effects engine
 * built on the Web Audio API — no files, no external assets, no network
 * dependencies, no libraries. Everything is generated from oscillators,
 * noise buffers and envelopes, with real musical structure (scales,
 * chords, basslines, percussion and layered lead parts).
 *
 * Architecture:
 *  - ONE lazily-created AudioContext, shared by music and SFX, reused for
 *    the whole session; siAudioDispose() closes it on teardown.
 *  - Two independent buses: music -> musicGain -> master -> destination
 *                        sfx  -> sfxGain  -> master -> destination
 *    so music and effect volumes/mutes are fully independent.
 *  - Music is a look-ahead scheduler (setInterval 60ms, horizon 0.2s)
 *    driving stateful "tracks" (menu / gameplay / victory / defeat).
 *    Switching states never restarts the context or duplicates playback;
 *    each track owns its own step counter, tempo and layer mix.
 *  - Gameplay music has three intensity tiers (rising with the wave
 *    number); tier changes are smooth crossfades of layer gains, not
 *    restarts.
 *  - SFX are one-shot synth voices with per-effect cooldowns so rapid
 *    auto-fire can never stack into an uncontrollable wall of sound.
 *  - Autoplay-safe: the context is created/resumed only inside a user
 *    gesture (Start Game, controls, or any key/touch on the game view).
 *  - Preferences (volumes + mutes) persist in localStorage under
 *    'si.audio.v1' and are validated on load; invalid values fall back to
 *    safe defaults.
 */
const SIA = {
  ctx: null,                // the single AudioContext (lazy)
  master: null,             // master gain (both buses)
  musicGain: null,          // music bus gain  (Phase 4: independent volume)
  sfxGain: null,            // effects bus gain (Phase 4: independent volume)
  unlocked: false,          // true once a user gesture allowed audio
  seqTimer: 0,              // scheduler interval id
  nextNoteAt: 0,            // when the next 16th-note grid tick is due
  track: null,              // current track id ('menu'|'gameplay'|...)
  prevTrack: null,          // last music track before a jingle interrupted
  jingleUntil: 0,           // ctx-time after which gameplay music resumes
  step: 0,                  // position in the current track's pattern
  bpm: 112,                 // current tempo (varies per track)
  intensity: 1,             // gameplay intensity tier (1..3, Phase 3)
  layers: {},               // per-layer gain nodes for smooth tier fades
  pending: [],              // scheduled stoppers for teardown cleanup
  sfxLast: Object.create(null), // per-effect cooldown bookkeeping
};

/* ---- Preferences (localStorage, validated) -------------------------- */
const SI_AUDIO_KEY = 'si.audio.v1';
function siAudioDefaults() { return { musicVol: 0.6, sfxVol: 0.7, musicOn: true, sfxOn: true }; }
function siNum(v, dflt, lo, hi) {
  const n = Number(v);
  return isFinite(n) && n >= lo && n <= hi ? n : dflt;
}
/** Load persisted audio settings; any missing/invalid field falls back to
 *  its default, so a corrupt store can never break the game. */
function siAudioLoadPrefs() {
  const d = siAudioDefaults();
  try {
    const raw = localStorage.getItem(SI_AUDIO_KEY);
    if (!raw) return d;
    const o = JSON.parse(raw);
    if (!o || typeof o !== 'object') return d;
    return {
      musicVol: siNum(o.musicVol, d.musicVol, 0, 1),
      sfxVol: siNum(o.sfxVol, d.sfxVol, 0, 1),
      musicOn: typeof o.musicOn === 'boolean' ? o.musicOn : d.musicOn,
      sfxOn: typeof o.sfxOn === 'boolean' ? o.sfxOn : d.sfxOn,
    };
  } catch { return d; }
}
function siAudioSavePrefs(p) {
  try { localStorage.setItem(SI_AUDIO_KEY, JSON.stringify(p)); } catch { /* private mode */ }
}
SIA.prefs = siAudioLoadPrefs();

/* ---- Context & buses ------------------------------------------------- */
/** Lazily create the single audio graph. Returns true if usable. */
function siAudioInit() {
  if (SIA.ctx) return true;
  const AC = window.AudioContext || window.webkitAudioContext;
  if (!AC) return false;
  try {
    SIA.ctx = new AC();
    SIA.master = SIA.ctx.createGain();
    SIA.master.gain.value = 0.9;
    SIA.master.connect(SIA.ctx.destination);
    SIA.musicGain = SIA.ctx.createGain();
    SIA.musicGain.gain.value = SIA.prefs.musicOn ? SIA.prefs.musicVol * 0.5 : 0;
    SIA.musicGain.connect(SIA.master);
    SIA.sfxGain = SIA.ctx.createGain();
    SIA.sfxGain.gain.value = SIA.prefs.sfxOn ? SIA.prefs.sfxVol * 0.9 : 0;
    SIA.sfxGain.connect(SIA.master);
    // Shared white-noise buffer for percussion and explosions.
    const len = Math.floor(SIA.ctx.sampleRate * 0.5);
    SIA.noise = SIA.ctx.createBuffer(1, len, SIA.ctx.sampleRate);
    const ch = SIA.noise.getChannelData(0);
    for (let i = 0; i < len; i++) ch[i] = Math.random() * 2 - 1;
    return true;
  } catch { SIA.ctx = null; return false; }
}

/** Unlock/resume audio from inside a user gesture. Safe to call often. */
function siAudioUnlock() {
  if (!siAudioInit()) return;
  SIA.unlocked = true;
  if (SIA.ctx.state === 'suspended') SIA.ctx.resume().catch(() => {});
}

/* ---- Voice primitives -------------------------------------------------
 * Short helpers that build oscillator/noise voices with envelopes. Every
 * node is scheduled to stop itself, so nothing leaks; teardown also keeps
 * a defensive pending-stopper list. */
function siTone(bus, opts) {
  if (!SIA.ctx || !bus) return;
  const t0 = opts.at != null ? opts.at : SIA.ctx.currentTime;
  const dur = Math.max(0.01, opts.dur || 0.15);
  const osc = SIA.ctx.createOscillator();
  osc.type = opts.type || 'square';
  osc.frequency.setValueAtTime(Math.max(20, opts.f0 || 220), t0);
  if (opts.f1 && opts.f1 !== opts.f0) {
    if (opts.exp) osc.frequency.exponentialRampToValueAtTime(Math.max(20, opts.f1), t0 + dur);
    else osc.frequency.linearRampToValueAtTime(Math.max(20, opts.f1), t0 + dur);
  }
  const g = SIA.ctx.createGain();
  const vol = opts.vol != null ? opts.vol : 0.2;
  const atk = Math.min(0.02, dur * 0.25);
  g.gain.setValueAtTime(0.0001, t0);
  g.gain.exponentialRampToValueAtTime(vol, t0 + atk);
  if (opts.sustain != null) {
    g.gain.setValueAtTime(vol, t0 + dur * opts.sustain);
    g.gain.exponentialRampToValueAtTime(0.0001, t0 + dur);
  } else {
    g.gain.exponentialRampToValueAtTime(0.0001, t0 + dur);
  }
  let out = g;
  if (opts.filter) {
    const f = SIA.ctx.createBiquadFilter();
    f.type = opts.filter;
    if (opts.cutoff) f.frequency.value = opts.cutoff;
    if (opts.q) f.Q.value = opts.q;
    g.connect(f); out = f;
  }
  osc.connect(g); out.connect(bus);
  osc.start(t0); osc.stop(t0 + dur + 0.03);
  osc.onended = () => { try { osc.disconnect(); g.disconnect(); } catch {} };
}
function siNoise(bus, opts) {
  if (!SIA.ctx || !bus || !SIA.noise) return;
  const t0 = opts.at != null ? opts.at : SIA.ctx.currentTime;
  const dur = Math.max(0.01, opts.dur || 0.12);
  const src = SIA.ctx.createBufferSource();
  src.buffer = SIA.noise;
  src.loop = true;
  const f = SIA.ctx.createBiquadFilter();
  f.type = opts.filter || 'lowpass';
  f.frequency.setValueAtTime(opts.cutoff || 1200, t0);
  if (opts.cutoffEnd) f.frequency.exponentialRampToValueAtTime(Math.max(40, opts.cutoffEnd), t0 + dur);
  if (opts.q) f.Q.value = opts.q;
  const g = SIA.ctx.createGain();
  const vol = opts.vol != null ? opts.vol : 0.2;
  g.gain.setValueAtTime(0.0001, t0);
  g.gain.exponentialRampToValueAtTime(vol, t0 + Math.min(0.015, dur * 0.3));
  g.gain.exponentialRampToValueAtTime(0.0001, t0 + dur);
  src.connect(f); f.connect(g); g.connect(bus);
  src.start(t0); src.stop(t0 + dur + 0.03);
  src.onended = () => { try { src.disconnect(); f.disconnect(); g.disconnect(); } catch {} };
}
/** A pad voice: two detuned saws through a lowpass, slow attack/release. */
function siPad(bus, freq, at, dur, vol, cutoff) {
  if (!SIA.ctx || !bus) return;
  const g = SIA.ctx.createGain();
  g.gain.setValueAtTime(0.0001, at);
  g.gain.linearRampToValueAtTime(vol, at + dur * 0.35);
  g.gain.linearRampToValueAtTime(0.0001, at + dur);
  const f = SIA.ctx.createBiquadFilter();
  f.type = 'lowpass'; f.frequency.value = cutoff || 900; f.Q.value = 0.7;
  const oscs = [];
  for (const det of [-7, 0, 7]) {
    const o = SIA.ctx.createOscillator();
    o.type = 'sawtooth';
    o.frequency.value = freq;
    o.detune.value = det;
    o.connect(f);
    o.start(at); o.stop(at + dur + 0.05);
    oscs.push(o);
  }
  f.connect(g); g.connect(bus);
  const end = at + dur + 0.1;
  setTimeout(() => { try { for (const o of oscs) o.disconnect(); f.disconnect(); g.disconnect(); } catch {} },
    Math.max(0, (end - SIA.ctx.currentTime) * 1000) + 60);
}

/* ---- Music theory helpers -------------------------------------------- */
const NOTE = (() => {
  const names = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
  function hz(n) { return 440 * Math.pow(2, (n - 69) / 12); }   // MIDI -> Hz
  function midi(name, oct) {
    const m = /^([A-G])(#|b)?$/.exec(name);
    if (!m) return 60;
    let v = names.indexOf(m[1]);
    if (m[2] === '#') v += 1;
    if (m[2] === 'b') v -= 1;
    return v + (oct + 1) * 12;
  }
  return { hz, midi, f: (name, oct) => hz(midi(name, oct)) };
})();
/* A-minor pentatonic-ish scale used across tracks for coherence. */
const SI_SCALE = ['A', 'C', 'D', 'E', 'G'];

/* ---- Tracks -----------------------------------------------------------
 * Each track defines: bpm, bars (length in 16th steps), and layer
 * functions invoked per step. Layer levels are gated by `intensity` so
 * higher waves add parts instead of restarting anything. */
const SI_TRACKS = {
  /* MENU: atmospheric ambient — slow pads over a sub bass pulse. */
  menu: {
    bpm: 72,
    bars: 32,
    layers: {
      pad:   { gain: 0.30, tier: 1 },
      sub:   { gain: 0.26, tier: 1 },
      shimmer:{ gain: 0.10, tier: 1 },
    },
    step(at, s, L) {
      const barLen = 16;
      const bar = Math.floor(s / barLen) % 8;
      // Chord progression Am - F - C - G (i - VI - III - VII), 2 bars each.
      const roots = [NOTE.f('A', 2), NOTE.f('F', 2), NOTE.f('C', 3), NOTE.f('G', 2)];
      const root = roots[Math.floor(bar / 2)];
      if (s % barLen === 0) {
        if (L.pad) siPad(L.pad, root, at, 3.2, 0.16, 750);
        if (L.pad) siPad(L.pad, root * 1.5, at, 3.2, 0.10, 900);       // fifth
        if (L.pad && bar % 2 === 0) siPad(L.pad, root * 2, at, 3.2, 0.06, 1400);
      }
      if (L.sub && s % 8 === 0) siTone(L.sub, { f0: root / 2, dur: 1.5, type: 'sine', vol: 0.22, sustain: 0.5 });
      if (L.shimmer && s % 4 === 2) {
        const deg = SI_SCALE[(Math.floor(s / 4) + bar) % SI_SCALE.length];
        siTone(L.shimmer, { f0: NOTE.f(deg, 5), dur: 0.5, type: 'triangle', vol: 0.05, filter: 'bandpass', cutoff: 1800, q: 2 });
      }
    },
  },

  /* GAMEPLAY: energetic synthwave — driving bass, kick/snare/hat groove,
   * arpeggio and a lead melody that develops across 8 bars. Intensity
   * tiers add layers (arp at 1, lead at 2, extra hats+stab at 3). */
  gameplay: {
    bpm: 124,
    bars: 128,
    layers: {
      bass:  { gain: 0.34, tier: 1 },
      drums: { gain: 0.42, tier: 1 },
      arp:   { gain: 0.20, tier: 1 },
      lead:  { gain: 0.24, tier: 2 },
      stab:  { gain: 0.18, tier: 3 },
    },
    step(at, s, L) {
      const b = s % 16;                          // position in the bar
      const barIdx = Math.floor(s / 16) % 8;     // 8-bar melodic structure
      // Progression: Am - F - C - G (two bars each) in A minor.
      const prog = [
        ['A', 2], ['A', 2], ['F', 2], ['F', 2],
        ['C', 3], ['C', 3], ['G', 2], ['G', 2],
      ][barIdx];
      const rootHz = NOTE.f(prog[0], prog[1]);

      /* Drums: four-on-the-floor kick, snare on 2&4, offbeat hats. */
      if (L.drums) {
        if (b % 4 === 0) siNoise(L.drums, { dur: 0.14, cutoff: 4200, cutoffEnd: 120, vol: 0.5, filter: 'lowpass' });
        if (b === 4 || b === 12) siNoise(L.drums, { dur: 0.12, cutoff: 1800, cutoffEnd: 900, vol: 0.3, filter: 'bandpass', q: 0.8 });
        if (b % 2 === 1) siNoise(L.drums, { dur: 0.05, cutoff: 8000, vol: 0.12, filter: 'highpass' });
        if (SIA.intensity >= 3 && b % 2 === 0 && b % 4 !== 0)
          siNoise(L.drums, { dur: 0.03, cutoff: 9000, vol: 0.08, filter: 'highpass' });
      }
      /* Bassline: syncopated octave root pattern. */
      if (L.bass) {
        const pat = [0, 3, 6, 10, 12, 14];
        if (pat.includes(b)) {
          const oct = (b === 6 || b === 14) ? 2 : 1;
          siTone(L.bass, { f0: rootHz * oct, dur: 0.16, type: 'sawtooth', vol: 0.26, filter: 'lowpass', cutoff: 700 });
        }
      }
      /* Arpeggio: 16th-note up-down over the current chord. */
      if (L.arp) {
        const chord = [rootHz, rootHz * 1.189, rootHz * 1.498];   // minor-ish
        const seq = [0, 1, 2, 1];
        siTone(L.arp, { f0: chord[seq[b % 4]] * 2, dur: 0.09, type: 'square', vol: 0.07, filter: 'lowpass', cutoff: 2600 });
      }
      /* Lead melody: a written 8-bar phrase (MIDI note numbers, A minor).
       * 0 = rest. Distinct melodic contour, not a beep loop. */
      if (L.lead) {
        const MEL = [
          69, 0, 72, 0, 74, 0, 76, 0,  72, 0, 69, 0, 67, 0, 69, 0,
          65, 0, 69, 0, 72, 0, 74, 0,  76, 0, 79, 0, 76, 0, 74, 0,
          72, 0, 76, 0, 79, 0, 76, 0,  74, 0, 72, 0, 69, 0, 67, 0,
          67, 0, 71, 0, 74, 0, 71, 0,  79, 0, 78, 0, 76, 0, 74, 0,
          69, 0, 72, 0, 76, 0, 79, 0,  81, 0, 79, 0, 76, 0, 74, 0,
          77, 0, 76, 0, 74, 0, 72, 0,  74, 0, 72, 0, 69, 0, 72, 0,
          76, 0, 79, 0, 84, 0, 79, 0,  76, 0, 74, 0, 72, 0, 74, 0,
          72, 0, 69, 0, 67, 0, 64, 0,  69, 0, 0, 0, 0, 0, 0, 0,
        ];
        const n = MEL[s % MEL.length];
        if (n) siTone(L.lead, { f0: NOTE.hz(n), dur: 0.20, type: 'sawtooth', vol: 0.13, filter: 'lowpass', cutoff: 3200, sustain: 0.4 });
      }
      /* Tier-3 stab: chordal accent on the downbeats. */
      if (L.stab && b === 0) {
        for (const mul of [1, 1.189, 1.498])
          siTone(L.stab, { f0: rootHz * 2 * mul, dur: 0.22, type: 'sawtooth', vol: 0.06, filter: 'lowpass', cutoff: 2400 });
      }
    },
  },

  /* LEVEL COMPLETE: short victory jingle (handled as a jingle, below). */
  victory: {
    bpm: 132,
    bars: 16,
    layers: { lead: { gain: 0.3, tier: 1 }, bass: { gain: 0.24, tier: 1 } },
    step(at, s, L) {
      // Bright ascending fanfare: A - C - E - A, then a resolving figure.
      const FAN = [
        [69, 0], [69, 1], [72, 1], [76, 1], [81, 2],
        [79, 1], [76, 1], [81, 3],
      ];
      const idx = Math.floor(s / 2);
      if (s % 2 === 0 && idx < FAN.length) {
        const [n, len] = FAN[idx];
        const dur = 0.14 * len + 0.08;
        if (L.lead) siTone(L.lead, { f0: NOTE.hz(n), dur, type: 'square', vol: 0.16, sustain: 0.5 });
        if (L.lead) siTone(L.lead, { f0: NOTE.hz(n + 12), dur, type: 'triangle', vol: 0.07, sustain: 0.5 });
      }
      if (L.bass && s % 4 === 0) {
        const bassN = [45, 48, 52, 57][Math.floor(s / 4) % 4];
        siTone(L.bass, { f0: NOTE.hz(bassN), dur: 0.3, type: 'triangle', vol: 0.2 });
      }
    },
  },

  /* GAME OVER: dramatic descending defeat sequence, then silence. */
  defeat: {
    bpm: 96,
    bars: 16,
    layers: { lead: { gain: 0.3, tier: 1 }, pad: { gain: 0.3, tier: 1 }, bass: { gain: 0.26, tier: 1 } },
    step(at, s, L) {
      // Cinematic descent: A4 -> F4 -> D4 -> B3 -> A3, slow and heavy.
      const DESC = [69, 65, 62, 59, 57];
      if (s % 4 === 0) {
        const n = DESC[Math.floor(s / 4) % DESC.length];
        if (L.lead) siTone(L.lead, { f0: NOTE.hz(n), dur: 0.7, type: 'sawtooth', vol: 0.14, filter: 'lowpass', cutoff: 1600, sustain: 0.4 });
        if (L.bass) siTone(L.bass, { f0: NOTE.hz(n - 24), dur: 0.9, type: 'sine', vol: 0.2, sustain: 0.5 });
      }
      if (L.pad && s === 0) siPad(L.pad, NOTE.f('A', 2), at, 3.4, 0.12, 600);
      if (L.pad && s === 8) siPad(L.pad, NOTE.f('F', 2), at, 3.0, 0.10, 500);
    },
  },
};

/* ---- Music engine ---------------------------------------------------- */
/** Build (or rebuild) the per-layer gain nodes for a track. Layer gains
 *  are faded in/out over ~0.8s so track and intensity switches are smooth
 *  rather than clicks or restarts. */
function siMusicLayers(trackId) {
  const def = SI_TRACKS[trackId];
  if (!def || !SIA.ctx) return null;
  const now = SIA.ctx.currentTime;
  const keep = {};
  for (const [name, cfg] of Object.entries(def.layers)) {
    let g = SIA.layers[name];
    if (!g) {
      g = SIA.ctx.createGain();
      g.gain.value = 0;
      g.connect(SIA.musicGain);
      SIA.layers[name] = g;
    }
    keep[name] = g;
    const target = SIA.intensity >= cfg.tier ? cfg.gain : 0;
    g.gain.cancelScheduledValues(now);
    g.gain.setTargetAtTime(target, now, 0.25);
  }
  // Fade out + disconnect layers not used by this track.
  for (const [name, g] of Object.entries(SIA.layers)) {
    if (keep[name]) continue;
    g.gain.cancelScheduledValues(now);
    g.gain.setTargetAtTime(0, now, 0.15);
    setTimeout(() => { try { g.disconnect(); } catch {} }, 900);
    delete SIA.layers[name];
  }
  return keep;
}

/** The look-ahead scheduler. Runs every 60ms while music is active. */
function siMusicSchedule() {
  if (!SIA.seqTimer || !SIA.ctx) return;
  const def = SI_TRACKS[SIA.track];
  if (!def) return;
  const now = SIA.ctx.currentTime;
  // Jingles (victory/defeat) hand control back when their bars end.
  if (SIA.jingleUntil && now >= SIA.jingleUntil) {
    SIA.jingleUntil = 0;
    const back = SIA.prevTrack || 'gameplay';
    SIA.prevTrack = null;
    // If the game ended meanwhile, fade to silence instead of resuming.
    if (!SIG.g || SIG.g.state === 'over') { siMusicStop(); return; }
    siMusicPlay(back, { keepStep: false });
    return;
  }
  const stepDur = 60 / def.bpm / 4;               // 16th-note grid
  const ahead = now + 0.20;
  const L = SIA.layerNodes;
  while (SIA.nextNoteAt < ahead) {
    def.step(SIA.nextNoteAt, SIA.step, L);
    SIA.step = (SIA.step + 1) % def.bars;
    SIA.nextNoteAt += stepDur;
  }
}

/** Start (or switch to) a track without duplicating the scheduler. */
function siMusicPlay(trackId, opts) {
  if (!siAudioInit()) return;
  if (SIA.ctx.state === 'suspended') SIA.ctx.resume().catch(() => {});
  if (!SI_TRACKS[trackId]) trackId = 'gameplay';
  const switching = SIA.track !== trackId;
  SIA.track = trackId;
  SIA.layerNodes = siMusicLayers(trackId);
  if (switching && !(opts && opts.keepStep)) SIA.step = 0;
  if (!SIA.seqTimer) {
    SIA.nextNoteAt = SIA.ctx.currentTime + 0.06;
    SIA.seqTimer = setInterval(siMusicSchedule, 60);
  }
}
function siMusicStop() {
  if (SIA.seqTimer) { clearInterval(SIA.seqTimer); SIA.seqTimer = 0; }
  if (SIA.ctx) {
    const now = SIA.ctx.currentTime;
    for (const [, g] of Object.entries(SIA.layers)) {
      g.gain.cancelScheduledValues(now);
      g.gain.setTargetAtTime(0, now, 0.1);
    }
  }
  SIA.track = null; SIA.jingleUntil = 0; SIA.prevTrack = null;
}

/** Play a one-shot jingle over the music bus, then resume gameplay music.
 *  `secs` is how long the jingle owns the soundtrack. */
function siMusicJingle(kind, secs) {
  if (!SIA.ctx || !SIA.prefs.musicOn) return;
  SIA.prevTrack = SIA.track === kind ? (SIA.prevTrack || 'gameplay') : (SIA.track || 'gameplay');
  SIA.jingleUntil = SIA.ctx.currentTime + secs;
  siMusicPlay(kind, { keepStep: false });
}

/** Phase 3: pick the gameplay intensity tier from the wave number. Tiers
 *  rise smoothly with progress: 1 (waves 1-2), 2 (3-5), 3 (6+). Called on
 *  level transitions; changing tier only re-mixes layer gains. */
function siMusicIntensity(wave) {
  const tier = wave >= 6 ? 3 : wave >= 3 ? 2 : 1;
  if (tier === SIA.intensity) return;
  SIA.intensity = tier;
  if (SIA.track === 'gameplay' && SIA.seqTimer) {
    SIA.layerNodes = siMusicLayers('gameplay');
  }
}

/** Full disposal: stop the loop, fade the buses, close the context.
 *  Called from siTeardown so leaving the route can never leak audio. */
function siAudioDispose() {
  siMusicStop();
  if (SIA.ctx) {
    try { SIA.ctx.close(); } catch { /* already closed */ }
  }
  for (const [, g] of Object.entries(SIA.layers)) { try { g.disconnect(); } catch {} }
  SIA.layers = {}; SIA.layerNodes = null;
  SIA.ctx = SIA.master = SIA.musicGain = SIA.sfxGain = SIA.noise = null;
  SIA.seqTimer = 0; SIA.step = 0; SIA.track = null;
  SIA.jingleUntil = 0; SIA.prevTrack = null; SIA.intensity = 1;
  SIA.unlocked = false;
}

/** Keep the soundtrack aligned with the game state. Called from siFrame
 *  whenever state may have changed; it is idempotent, so calling it every
 *  frame can never duplicate playback.
 *   ready/menu  -> menu track
 *   playing/between -> gameplay track (with victory jingle during 'between')
 *   paused      -> gameplay track continues (ducked) — see siPauseMusic
 *   over        -> defeat jingle, then silence (music stops when it ends) */
function siMusicEnsure() {
  // A jingle (victory/defeat) owns the soundtrack until it hands back;
  // never let the per-frame call stomp over it.
  if (SIA.jingleUntil && SIA.ctx && SIA.ctx.currentTime < SIA.jingleUntil) return;
  if (!SIA.unlocked || !SIA.prefs.musicOn) { siMusicStop(); return; }
  const g = SIG.g;
  if (!g) { siMusicStop(); return; }
  if (g.state === 'over') { siMusicStop(); return; }   // defeat fades itself out
  if (g.state === 'ready') { siMusicPlay('menu'); return; }
  if (g.state === 'paused') {                     // ducked but alive
    if (SIA.track !== 'gameplay' && SIA.track !== 'menu') siMusicPlay('menu');
    return;
  }
  // playing | between: gameplay soundtrack (jingle handled at transition)
  siMusicPlay('gameplay');
  siMusicIntensity(g.wave);
}

/** Pause ducking: lower the music bus while paused, restore on resume. */
function siPauseMusic(paused) {
  if (!SIA.ctx || !SIA.musicGain) return;
  const now = SIA.ctx.currentTime;
  const base = SIA.prefs.musicOn ? SIA.prefs.musicVol * 0.5 : 0;
  SIA.musicGain.gain.cancelScheduledValues(now);
  SIA.musicGain.gain.setTargetAtTime(paused ? base * 0.35 : base, now, 0.2);
}

/* ---- Sound effects (Phase 4) -----------------------------------------
 * Each effect has its own character (laser sweep, noise burst, arpeggio,
 * metallic ring…), its own cooldown, and respects the independent SFX
 * volume/mute. Cooldowns prevent auto-fire from stacking into mush. */
const SI_SFX_COOLDOWN = { shoot: 90, kill: 60, hit: 150, shieldBlock: 200 };

function siSfxAllowed(kind, nowMs) {
  if (!SIA.ctx || !SIA.prefs.sfxOn) return false;
  const cd = SI_SFX_COOLDOWN[kind] || 0;
  const last = SIA.sfxLast[kind] || 0;
  if (cd && nowMs - last < cd) return false;
  SIA.sfxLast[kind] = nowMs;
  return true;
}

function siSfx(kind) {
  if (!SIA.ctx) return;
  const now = performance.now();
  const bus = SIA.sfxGain;
  const t = SIA.ctx.currentTime;
  switch (kind) {
    case 'shoot':          // punchy laser: fast upward chirp + tiny noise
      if (!siSfxAllowed('shoot', now)) return;
      siTone(bus, { f0: 620, f1: 1180, dur: 0.09, type: 'square', vol: 0.12, exp: true });
      siTone(bus, { f0: 1240, f1: 620, dur: 0.06, type: 'sawtooth', vol: 0.05, exp: true });
      break;
    case 'kill':           // layered explosion: noise burst + pitch drop
      if (!siSfxAllowed('kill', now)) return;
      siNoise(bus, { dur: 0.22, cutoff: 2600, cutoffEnd: 160, vol: 0.34, filter: 'lowpass' });
      siTone(bus, { f0: 300, f1: 70, dur: 0.2, type: 'triangle', vol: 0.22, exp: true });
      break;
    case 'hit':            // player damage: impact + descending pitch
      if (!siSfxAllowed('hit', now)) return;
      siNoise(bus, { dur: 0.3, cutoff: 1400, cutoffEnd: 90, vol: 0.4, filter: 'lowpass' });
      siTone(bus, { f0: 420, f1: 60, dur: 0.38, type: 'sawtooth', vol: 0.26, exp: true });
      siTone(bus, { f0: 210, f1: 50, dur: 0.42, type: 'square', vol: 0.14, exp: true });
      break;
    case 'power':          // generic power-up acquired: bright ascending arpeggio
      siTone(bus, { f0: 523.25, dur: 0.09, type: 'square', vol: 0.14 });
      siTone(bus, { f0: 659.25, dur: 0.09, type: 'square', vol: 0.14, at: t + 0.07 });
      siTone(bus, { f0: 783.99, dur: 0.09, type: 'square', vol: 0.14, at: t + 0.14 });
      siTone(bus, { f0: 1046.5, dur: 0.16, type: 'square', vol: 0.14, at: t + 0.21 });
      break;
    case 'rapid':          // fast energetic activation: quick trill
      for (let i = 0; i < 6; i++)
        siTone(bus, { f0: 700 + i * 140, dur: 0.05, type: 'square', vol: 0.11, at: t + i * 0.045 });
      break;
    case 'triple':         // three-layer projectile: three stacked chirps
      siTone(bus, { f0: 520, f1: 900, dur: 0.1, type: 'sawtooth', vol: 0.12, exp: true });
      siTone(bus, { f0: 660, f1: 1100, dur: 0.1, type: 'square', vol: 0.1, exp: true, at: t + 0.05 });
      siTone(bus, { f0: 800, f1: 1300, dur: 0.1, type: 'triangle', vol: 0.1, exp: true, at: t + 0.1 });
      break;
    case 'shield':         // futuristic energy shield: rising airy sweep
      siTone(bus, { f0: 240, f1: 980, dur: 0.5, type: 'sine', vol: 0.16, exp: true });
      siTone(bus, { f0: 360, f1: 1470, dur: 0.5, type: 'triangle', vol: 0.1, exp: true });
      siNoise(bus, { dur: 0.45, cutoff: 5200, cutoffEnd: 1200, vol: 0.06, filter: 'bandpass', q: 3 });
      break;
    case 'shieldBlock':    // metallic energy impact: ring + clank
      if (!siSfxAllowed('shieldBlock', now)) return;
      siTone(bus, { f0: 1560, f1: 1180, dur: 0.22, type: 'square', vol: 0.16, exp: true });
      siTone(bus, { f0: 2340, f1: 1950, dur: 0.18, type: 'sine', vol: 0.1, exp: true });
      siNoise(bus, { dur: 0.12, cutoff: 6000, cutoffEnd: 1500, vol: 0.16, filter: 'bandpass', q: 4 });
      break;
    case 'double':         // deep powerful activation: sub swell
      siTone(bus, { f0: 110, f1: 55, dur: 0.5, type: 'sine', vol: 0.3, exp: true });
      siTone(bus, { f0: 220, f1: 82, dur: 0.4, type: 'sawtooth', vol: 0.14, exp: true, filter: 'lowpass', cutoff: 500 });
      siNoise(bus, { dur: 0.3, cutoff: 900, cutoffEnd: 120, vol: 0.14, filter: 'lowpass' });
      break;
    case 'life':           // positive arcade reward melody
      [[659.25, 0], [783.99, 0.09], [987.77, 0.18], [1318.5, 0.27]].forEach(([f, d]) =>
        siTone(bus, { f0: f, dur: 0.14, type: 'square', vol: 0.13, at: t + d }));
      break;
    case 'slow':           // slow motion: descending pitch + atmosphere
      siTone(bus, { f0: 900, f1: 220, dur: 0.8, type: 'sine', vol: 0.16, exp: true });
      siTone(bus, { f0: 1350, f1: 330, dur: 0.8, type: 'triangle', vol: 0.08, exp: true });
      siNoise(bus, { dur: 0.7, cutoff: 3000, cutoffEnd: 400, vol: 0.05, filter: 'lowpass' });
      break;
    case 'levelDone':      // short victory flourish (Level complete)
      [[523.25, 0], [659.25, 0.1], [783.99, 0.2], [1046.5, 0.3]].forEach(([f, d]) =>
        siTone(bus, { f0: f, dur: 0.18, type: 'square', vol: 0.15, at: t + d }));
      siNoise(bus, { dur: 0.4, cutoff: 7000, cutoffEnd: 2500, vol: 0.08, filter: 'highpass', at: t + 0.3 });
      break;
    case 'transition':     // smooth transition: soft sweep into the next level
      siTone(bus, { f0: 330, f1: 660, dur: 0.6, type: 'sine', vol: 0.12, exp: true });
      siTone(bus, { f0: 495, f1: 990, dur: 0.6, type: 'triangle', vol: 0.07, exp: true });
      break;
    case 'over':           // dramatic defeat: descending doom figure
      siTone(bus, { f0: 392, f1: 196, dur: 0.9, type: 'sawtooth', vol: 0.2, exp: true, filter: 'lowpass', cutoff: 900 });
      siTone(bus, { f0: 293.66, f1: 146.83, dur: 1.1, type: 'square', vol: 0.12, exp: true, at: t + 0.15 });
      siNoise(bus, { dur: 1.0, cutoff: 1200, cutoffEnd: 60, vol: 0.22, filter: 'lowpass' });
      break;
  }
}

/* ---- Settings UI (Phase 4) ------------------------------------------- */
/** Apply prefs to the live gain nodes immediately (no restart needed). */
function siAudioApplyPrefs() {
  if (!SIA.ctx) return;
  const now = SIA.ctx.currentTime;
  if (SIA.musicGain) {
    const v = SIA.prefs.musicOn ? SIA.prefs.musicVol * 0.5 : 0;
    SIA.musicGain.gain.cancelScheduledValues(now);
    SIA.musicGain.gain.setTargetAtTime(v, now, 0.05);
  }
  if (SIA.sfxGain) {
    const v = SIA.prefs.sfxOn ? SIA.prefs.sfxVol * 0.9 : 0;
    SIA.sfxGain.gain.cancelScheduledValues(now);
    SIA.sfxGain.gain.setTargetAtTime(v, now, 0.05);
  }
}
/** Persist + apply + repaint the settings panel. */
function siAudioSetPrefs(patch) {
  Object.assign(SIA.prefs, patch);
  siAudioSavePrefs(SIA.prefs);
  siAudioApplyPrefs();
  siAudioSettingsPaint();
  siSoundButton();
}
/** Reflect prefs in the settings panel controls. */
function siAudioSettingsPaint() {
  const wrap = $('#siAudioPanel'); if (!wrap) return;
  const mv = $('#siMusicVol'), sv = $('#siSfxVol');
  if (mv) { mv.value = String(Math.round(SIA.prefs.musicVol * 100)); }
  if (sv) { sv.value = String(Math.round(SIA.prefs.sfxVol * 100)); }
  const mvv = $('#siMusicVolVal'), svv = $('#siSfxVolVal');
  if (mvv) mvv.textContent = Math.round(SIA.prefs.musicVol * 100) + '%';
  if (svv) svv.textContent = Math.round(SIA.prefs.sfxVol * 100) + '%';
  const mm = $('#siMusicMute'), sm = $('#siSfxMute');
  if (mm) {
    mm.setAttribute('aria-pressed', String(!SIA.prefs.musicOn));
    mm.classList.toggle('muted', !SIA.prefs.musicOn);
    mm.textContent = SIA.prefs.musicOn ? 'Music: on' : 'Music: muted';
  }
  if (sm) {
    sm.setAttribute('aria-pressed', String(!SIA.prefs.sfxOn));
    sm.classList.toggle('muted', !SIA.prefs.sfxOn);
    sm.textContent = SIA.prefs.sfxOn ? 'Effects: on' : 'Effects: muted';
  }
}

/** Public toggle kept for the HUD button: flips BOTH mutes together (a
 *  quick master switch), still fully independent in the settings panel. */
function siSoundToggle() {
  const anyOn = SIA.prefs.musicOn || SIA.prefs.sfxOn;
  siAudioSetPrefs({ musicOn: !anyOn, sfxOn: !anyOn });
  if (!anyOn) {
    siAudioUnlock();
    siMusicEnsure();
  } else {
    siMusicStop();
  }
}

/** Reflect the combined preference in the HUD button. */
function siSoundButton() {
  const btn = $('#siSoundBtn');
  if (!btn) return;
  const on = SIA.prefs.musicOn || SIA.prefs.sfxOn;
  btn.setAttribute('aria-pressed', String(on));
  btn.classList.toggle('on', on);
  btn.title = on ? 'Sound on — click to mute all audio' : 'Sound off — click to enable audio';
  btn.setAttribute('aria-label', btn.title);
  const lbl = $('#siSoundLbl');
  if (lbl) lbl.textContent = on ? 'sound on' : 'sound off';
}

function siInit(canvas) {
  SIG.g = SI.newGame();
  SIG.ctx = canvas.getContext('2d');
  SIG.reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  siResize();
}

/** Fit the canvas to its container at the correct device pixel ratio while
 *  keeping logical coordinates fixed at SI.W x SI.H. */
function siResize() {
  const canvas = $('#siCanvas'); if (!canvas) return;
  const wrap = canvas.parentElement;
  // In fullscreen the stage is height-constrained: fit the canvas to the
  // available height (capped by width) so nothing is cropped or stretched.
  let cssW, cssH;
  if (siIsFullscreen()) {
    // Available height for the STAGE = viewport minus the panel's other
    // rows (HUD, controls, notice, touch pads). Measured live so every
    // viewport/zoom combination fits without scrolling or clipping.
    const panel = $('#siPanel');
    let chromeH = 0;
    if (panel) {
      const pr = panel.getBoundingClientRect();
      for (const c of panel.children) {
        if (c === wrap || c === canvas) continue;
        const cr = c.getBoundingClientRect();
        if (cr.height > 0) chromeH += cr.height;
      }
      // plus the panel's own vertical padding + gaps between rows
      const cs = getComputedStyle(panel);
      chromeH += parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom);
      chromeH += (panel.children.length - 1) * parseFloat(cs.columnGap || cs.gap || '16');
    }
    const availH = Math.max(80, (panel ? panel.clientHeight : innerHeight) - chromeH);
    const availW = wrap.clientWidth || 0;
    if (availH > 40 && availW > 40) {
      // Largest 480x620 box that fits both dimensions.
      cssH = Math.min(availH, availW * (SI.H / SI.W));
      cssW = cssH * (SI.W / SI.H);
    } else {                                  // not laid out yet
      cssW = wrap.clientWidth; cssH = cssW * (SI.H / SI.W);
    }
  } else {
    cssW = wrap.clientWidth;
    cssH = Math.round(cssW * (SI.H / SI.W));
  }
  const dpr = window.devicePixelRatio || 1;
  canvas.style.width = cssW + 'px';
  canvas.style.height = cssH + 'px';
  canvas.width = Math.round(cssW * dpr);
  canvas.height = Math.round(cssH * dpr);
  // Scale once into the transform: all drawing stays in logical units and
  // collision math is untouched by any later resize.
  SIG.ctx.setTransform(cssW * dpr / SI.W, 0, 0, cssH * dpr / SI.H, 0, 0);
  SIG.cssW = cssW; SIG.cssH = cssH;
}

function siInput() {
  return {
    left: !!(SIG.keys.ArrowLeft || SIG.keys.a || SIG.touch.left),
    right: !!(SIG.keys.ArrowRight || SIG.keys.d || SIG.touch.right),
    fire: !!(SIG.keys[' '] || SIG.touch.fire),
  };
}

/* ---- Fullscreen ------------------------------------------------------
 * Real Fullscreen API on the game panel, not a CSS-only fake. The canvas
 * keeps its logical coordinate system (SI.W x SI.H) and letterboxes inside
 * whatever space the fullscreen element offers, so nothing stretches.
 *
 * Safety properties:
 *  - Only ever entered from a user gesture (button click).
 *  - A rejection (unsupported, denied) is caught and reported in the HUD,
 *    never thrown; the game stays fully playable inline.
 *  - `fullscreenchange` restores the normal dashboard layout on exit.
 *  - The listener is registered through siOn(), so it is removed with the
 *    rest when the view is destroyed — no leak, no stuck layout.
 *  - Exit works via the button, Escape, or the browser's own control. */

/* Fullscreen: shared FTFS system. These names are kept as the public SI
 * surface (used by the keyboard handler and the regression suites). */
function siFsElement() { return ftFsElement(); }
function siFsSupported() { return ftFsSupported(); }
/** True when the game is in native fullscreen OR the automatic CSS
 *  immersive fallback (both need the height-fitted canvas layout). */
function siIsFullscreen() {
  return ftIsFullscreen()
    || document.body.classList.contains('ft-playing');
}

/** Enter/leave real browser fullscreen for the game panel. */
async function siToggleFullscreen() {
  await ftToggleFullscreen('si', 'si-fs', '#siFsNotice');
}

/** Small non-blocking notice used for fullscreen feedback. */
function siFsNotice(msg) {
  ftFsNotice('#siFsNotice', msg, 'si');
}

/** Reflect the current fullscreen state on the button + panel class. */
function siFsReflect() {
  ftFsReflect('si', 'si-fs', '#siFsBtn');
}


/** The one and only frame callback. */
function siFrame(t) {
  if (SIG.epoch !== SIG.myEpoch) return;    // stale loop: stop silently
  const g = SIG.g;
  const dtRaw = (t - SIG.lastT) / 1000;
  // Clamp: after tab suspension dt can be enormous; never simulate a jump.
  const dt = Math.min(Math.max(dtRaw, 0), 0.05);
  SIG.lastT = t;

  // Sound effects are triggered by observed state changes so they fire
  // exactly once per event, and only while sound is enabled.
  const prevScore = g.score, prevLives = g.lives, prevState = g.state;
  const prevEnemies = g.enemies.length;
  const prevWave = SI_HUD_CACHE.waveShown;

  // 'between' is the Phase 1 LEVEL COMPLETE beat: keep simulating so the
  // automatic countdown runs; 'playing' is normal combat.
  if (g.state === 'playing' || g.state === 'between') SI.step(g, siInput(), dt);

  // A state change to 'over' during this step must surface immediately.
  // Phase 3: the defeat jingle owns the soundtrack from here on.
  if (g.state === 'over' && SI_HUD_CACHE.overlay !== 'over') {
    siOverlay('over'); siSfx('over');
    if (SIA.unlocked && SIA.prefs.musicOn) siMusicJingle('defeat', 4.2);
  }
  if (g.score > prevScore) siSfx('kill');
  if (g.lives < prevLives) siSfx('hit');
  // Phase 1/3: the next level began — transition sweep + intensity bump.
  if (g.state === 'playing' && prevState === 'playing' && g.wave !== prevWave) siSfx('transition');
  if (g.fired) { siSfx('shoot'); g.fired = false; }   // auto/manual shot fired
  // Phase 2/4: SHIELD absorbed a bomb — metallic energy impact.
  if (g.shieldBlocked) { siSfx('shieldBlock'); g.shieldBlocked = false; }
  // Keep the soundtrack aligned with the game state (menu/play/pause).
  siMusicEnsure();
  // Phase 3: LEVEL COMPLETE beat — victory jingle over the music bus,
  // which hands back to the gameplay track when it ends.
  if (g.state === 'between' && prevState === 'playing') {
    siSfx('levelDone');
    if (SIA.unlocked && SIA.prefs.musicOn) siMusicJingle('victory', 1.9);
  }
  // Phase 1: end the LEVEL COMPLETE notification the moment the beat is
  // over (game over, restart, or the next level starting) — this must run
  // BEFORE the wave announcement below so the two banners hand over
  // cleanly instead of the new one being hidden by the old one's cleanup.
  if (g.state !== 'between' && SI_HUD_CACHE.levelDone) {
    SI_HUD_CACHE.levelDone = false;
    siLevelHide();
  }
  // Announce a newly started wave exactly once.
  if (g.wave !== SI_HUD_CACHE.waveShown) { SI_HUD_CACHE.waveShown = g.wave; siWaveBanner(); }
  // Phase 1: the engine just started the next level by itself.
  if (g.justLeveled) { g.justLeveled = false; }
  // Phase 1: show the LEVEL COMPLETE notification for the whole beat.
  if (g.state === 'between' && !SI_HUD_CACHE.levelDone) {
    SI_HUD_CACHE.levelDone = true;
    siLevelDone();
  }
  // Phase 2: a power-up was just awarded by the engine — announce it.
  // Phase 4: each ability has its own activation sound.
  if (g.justGotPower) {
    const kind = g.justGotPower;
    g.justGotPower = null;
    siPowerBanner(kind);
    siSfx('power');
    siSfx(kind);            // rapid | triple | shield | double | life | slow
  }
  // Phase 2: temporaries that expired this step get an expiry flash.
  if (g.powerExpired) {
    for (const kind of g.powerExpired) siPowerExpired(kind);
    g.powerExpired = null;
  }
  siDraw(g);
  siHud(g);
  SIG.raf = requestAnimationFrame(siFrame);
}

/* ---- drawing (logical units) ---------------------------------------- */
function siDraw(g) {
  const ctx = SIG.ctx;
  ctx.clearRect(0, 0, SI.W, SI.H);

  /* starfield: deterministic positions from a seeded list, drawn every
   * frame (cheap) but only twinkling when motion is allowed. */
  if (!SIG.stars) {
    SIG.stars = [];
    let seed = 1337;
    const rnd = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
    for (let i = 0; i < 70; i++) {
      SIG.stars.push({ x: rnd() * SI.W, y: rnd() * SI.H, s: 0.6 + rnd() * 1.6, ph: rnd() * 6.28 });
    }
  }
  for (const st of SIG.stars) {
    const tw = SIG.reducedMotion ? 0.55 : (0.45 + 0.45 * Math.sin(g.elapsed * 1.7 + st.ph));
    ctx.globalAlpha = tw;
    ctx.fillStyle = '#8fa3bf';
    ctx.fillRect(st.x, st.y, st.s, st.s);
  }
  ctx.globalAlpha = 1;

  /* danger line */
  ctx.strokeStyle = 'rgba(255,107,129,.4)';
  ctx.setLineDash([6, 6]);
  ctx.beginPath();
  ctx.moveTo(0, SI.DANGER_Y);
  ctx.lineTo(SI.W, SI.DANGER_Y);
  ctx.stroke();
  ctx.setLineDash([]);

  /* enemies: chunky pixel-style ships per type */
  for (const e of g.enemies) siDrawEnemy(ctx, e);

  /* player: pixel ship with a bright core and wing cannons */
  const px = g.player.x, py = g.player.y, pw = g.player.w;
  ctx.fillStyle = '#5ef2c0';
  // wings
  ctx.fillRect(px, py + 10, 7, 7);
  ctx.fillRect(px + pw - 7, py + 10, 7, 7);
  // hull
  ctx.fillRect(px + 3, py + 6, pw - 6, 8);
  // bridge / cannon
  ctx.fillStyle = '#eafff6';
  ctx.fillRect(px + pw / 2 - 3, py, 6, 9);
  // engine glow
  ctx.fillStyle = 'rgba(94,242,192,.55)';
  ctx.fillRect(px + 5, py + 17, 5, 3);
  ctx.fillRect(px + pw - 10, py + 17, 5, 3);

  /* Phase 2: active power-up effects on the player's ship.
   * SHIELD draws a protective ring; RAPID FIRE brightens the cannon;
   * TRIPLE SHOT shows three muzzle dots. Drawn after the ship so they
   * sit on top, all in logical units, matching the existing identity. */
  const hasPow = k => !!(g.powers[k] && g.powers[k].active);
  if (hasPow('shield')) {
    const t = g.powers.shield;
    // Gentle pulse; fades out over the last 1.5s so expiry is readable.
    const fading = t.left < 1.5 ? Math.max(0.25, t.left / 1.5) : 1;
    const pulse = SIG.reducedMotion ? 0.8 : (0.75 + 0.25 * Math.sin(g.elapsed * 6));
    ctx.globalAlpha = fading * pulse;
    ctx.strokeStyle = '#c9f2ff';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.arc(px + pw / 2, py + 8, 26, 0, 6.283);
    ctx.stroke();
    ctx.globalAlpha = 1;
  }
  if (hasPow('rapid') || hasPow('triple')) {
    ctx.fillStyle = hasPow('rapid') ? 'rgba(255,215,186,.85)' : 'rgba(201,242,255,.85)';
    const mx = px + pw / 2;
    ctx.fillRect(mx - 2, py - 4, 4, 4);                       // centre muzzle
    if (hasPow('triple')) {                                   // wing muzzles
      ctx.fillRect(px + 4, py + 2, 3, 3);
      ctx.fillRect(px + pw - 7, py + 2, 3, 3);
    }
  }

  /* projectiles: bright cores with a soft trail for contrast.
   * Phase 2: DOUBLE DAMAGE shots get a warm gold core so the player can
   * read which bullets pierce. */
  for (const b of g.bullets) {
    if (b.dmg === 2) {
      ctx.fillStyle = 'rgba(255,215,186,.30)';
      ctx.fillRect(b.x - 1, b.y + b.h, b.w + 2, 8);
      ctx.fillStyle = '#ffd7ba';
      ctx.fillRect(b.x, b.y, b.w, b.h);
    } else {
      ctx.fillStyle = 'rgba(234,255,246,.28)';
      ctx.fillRect(b.x - 1, b.y + b.h, b.w + 2, 7);
      ctx.fillStyle = '#ffffff';
      ctx.fillRect(b.x, b.y, b.w, b.h);
    }
  }
  for (const b of g.bombs) {
    ctx.fillStyle = 'rgba(255,143,171,.26)';
    ctx.fillRect(b.x - 1, b.y - 7, b.w + 2, 7);
    ctx.fillStyle = '#ff8fab';
    ctx.fillRect(b.x, b.y, b.w, b.h);
  }

  /* explosions */
  for (const f of g.fx) {
    const p = f.t / f.life;
    ctx.globalAlpha = 1 - p;
    if (f.hitPlayer) {
      ctx.strokeStyle = '#ff6b81';
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(g.player.x + g.player.w / 2, g.player.y + 6, 6 + p * 26, 0, 6.283);
      ctx.stroke();
    } else if (f.shieldBlock) {
      // Phase 2: shield absorbed the hit — cool blue ripple, no life lost.
      ctx.strokeStyle = '#c9f2ff';
      ctx.lineWidth = 2;
      ctx.globalAlpha = (1 - p) * 0.9;
      ctx.beginPath();
      ctx.arc(g.player.x + g.player.w / 2, g.player.y + 6, 10 + p * 30, 0, 6.283);
      ctx.stroke();
      ctx.globalAlpha = 1;
    } else {
      ctx.fillStyle = f.pts >= 30 ? '#ffd7ba' : '#9bf2c0';
      const r = 3 + p * 12;
      ctx.fillRect(f.x + 12 - r / 2, f.y + 8 - r / 2, r, r);
    }
    ctx.globalAlpha = 1;
  }
}

function siDrawEnemy(ctx, e) {
  /* Sprites are defined on a coarse pixel grid and blitted with whole
   * "pixels", which keeps the chunky arcade look at any canvas scale.
   * Each type has a distinct silhouette so rows read clearly. */
  const SPRITES = {
    // top row — highest value: wide "saucer" with antennae
    a: [
      '...111111111111...',
      '.1111111111111111.',
      '11..1111111111..11',
      '111111111111111111',
      '..11..111111..11..',
      '....111....111....',
    ],
    // second row: crab-like with claws
    b: [
      '...111111111111...',
      '.1111111111111111.',
      '111111111111111111',
      '111..11111111..111',
      '111111111111111111',
      '..111........111..',
    ],
    // third row: squid with tentacles
    c: [
      '.....11111111.....',
      '...111111111111...',
      '..11111111111111..',
      '..11..111111..11..',
      '.11.11111111.1111.',
      '....11..11..11....',
    ],
    // bottom rows: small drone
    d: [
      '....11111111......',
      '..111111111111....',
      '.11111111111111...',
      '11.1111111111.1111',
      '111111111111111111',
      '..111........111..',
    ],
    e: [
      '....11111111......',
      '..111111111111....',
      '.11111111111111...',
      '11.1111111111.1111',
      '111111111111111111',
      '..111........111..',
    ],
  };
  const rows = SPRITES[e.type] || SPRITES.d;
  const px = e.w / 18;                     // logical size of one sprite pixel
  ctx.fillStyle = e.color;
  for (let r = 0; r < rows.length; r++) {
    const row = rows[r];
    for (let c = 0; c < row.length; c++) {
      if (row[c] === '1') {
        ctx.fillRect(e.x + c * px, e.y + r * px, Math.ceil(px), Math.ceil(px));
      }
    }
  }
}

/* ---- HUD (DOM, updated only when values change) ---------------------- */
const SI_HUD_CACHE = {};
function siHud(g) {
  const set = (id, v) => {
    if (SI_HUD_CACHE[id] !== v) { SI_HUD_CACHE[id] = v; const el = $('#' + id); if (el) el.textContent = v; }
  };
  set('siScore', String(g.score));
  set('siHigh', String(g.highScore));
  set('siLives', String(g.lives));
  set('siWave', String(g.wave));

  /* Phase 2: active-power chips. Rendered only when the set of actives or
   * any rounded remaining-time value changes, so the DOM is touched at
   * most a few times per second, not per frame. */
  siPowerChips(g);
}

/* Phase 2: one chip per ACTIVE ability, with a live countdown bar.
 * EXTRA LIFE has no timer (permanent), so its chip shows no bar.
 * Recently EXPIRED abilities render one final time with the .expired
 * class so the fade-out animation is actually visible — the engine's
 * expiry signal is recorded here (SIG.powerFlash) because the chip strip
 * is rebuilt in the same frame the power dies. */
function siPowerChips(g) {
  const wrap = $('#siPowers'); if (!wrap) return;
  // Prune expired flashes older than 500ms.
  const now = performance.now();
  for (const k of Object.keys(SIG.powerFlash || {})) {
    if (now - SIG.powerFlash[k] > 500) delete SIG.powerFlash[k];
  }
  // Build a small comparable signature: kind -> rounded seconds left,
  // plus any pending expiry flashes.
  const sig = [];
  for (const kind of Object.keys(g.powers)) {
    const t = g.powers[kind];
    if (t && t.active) sig.push(kind + ':' + Math.ceil(t.left));
  }
  for (const kind of Object.keys(SIG.powerFlash || {})) sig.push('!' + kind);
  const sigStr = sig.sort().join('|');
  if (SI_HUD_CACHE.powerSig === sigStr) return;      // nothing changed
  SI_HUD_CACHE.powerSig = sigStr;

  let html = '';
  for (const kind of Object.keys(g.powers)) {
    const t = g.powers[kind];
    if (!t || !t.active) continue;
    const p = SI.POWERS[kind];
    const pct = p.temp ? Math.max(0, Math.min(100, (t.left / t.total) * 100)) : 100;
    const secs = p.temp ? Math.ceil(t.left) + 's' : '';
    html += `<span class="si-pow" id="sip_${kind}" title="${p.label}${p.temp ? ' · ' + secs + ' left' : ''}">` +
      `<span class="si-pow-k">${p.label}</span>` +
      (p.temp ? `<span class="si-pow-t">${secs}</span>` +
               `<span class="si-pow-bar"><span class="si-pow-fill" style="width:${pct.toFixed(0)}%"></span></span>`
             : '') +
      `</span>`;
  }
  // Expiry ghosts: the chip that just ran out, flashing once more.
  for (const kind of Object.keys(SIG.powerFlash || {})) {
    if (g.powers[kind] && g.powers[kind].active) continue;   // re-granted
    const p = SI.POWERS[kind]; if (!p) continue;
    html += `<span class="si-pow expired" id="sip_${kind}">` +
      `<span class="si-pow-k">${p.label}</span></span>`;
  }
  wrap.innerHTML = html;
  wrap.classList.toggle('has-powers', !!html);
}

/* ---- overlays (Ready / Paused / Game Over / Wave banner) ------------- */
function siOverlay(kind) {
  const ov = $('#siOverlay'); if (!ov) return;
  SI_HUD_CACHE.overlay = kind;
  if (!kind) { ov.classList.add('hidden'); ov.setAttribute('aria-hidden', 'true'); return; }
  ov.classList.remove('hidden'); ov.setAttribute('aria-hidden', 'false');
  const M = {
    ready: ['Space Invaders',
            'Your ship fires automatically — just steer with ← → or A/D. Clear every wave before they reach the line.',
            'Start Game'],
    paused: ['Paused', 'Press P or Resume to continue', 'Resume'],
    over: ['Game Over', null, 'Play Again'],
  };
  const [title, sub, btn] = M[kind] || M.ready;
  $('#siOvTitle').textContent = title;
  $('#siOvSub').textContent =
    kind === 'over'
      ? `Final score ${g_finalScore()} · wave ${g_finalWave()} reached`
      : (sub || '');
  $('#siOvBtn').textContent = btn;
  $('#siOvBtn').dataset.kind = kind;
}
function g_finalScore() { return SIG.g ? SIG.g.score : 0; }
function g_finalWave() { return SIG.g ? SIG.g.wave : 1; }

/* Wave banner: shown briefly between waves.
 * `SIG.bannerGen` is a generation counter: every show bumps it, and each
 * pending hide-timer only acts if its generation is still current. This
 * keeps the shared banner element safe when a new message (e.g. LEVEL
 * COMPLETE) replaces one whose timer is still pending. */
function siBannerShow(text, cls) {
  const el = $('#siWaveBanner'); if (!el) return;
  SIG.bannerGen = (SIG.bannerGen || 0) + 1;
  el.textContent = text;
  el.classList.remove('level');
  if (cls) el.classList.add(cls);
  el.classList.add('show');
  const myGen = SIG.bannerGen, myEpoch = SIG.epoch;
  window.setTimeout(() => {
    if (myEpoch !== SIG.epoch || myGen !== SIG.bannerGen) return;
    el.classList.remove('show', 'level');
  }, 1400);
}
function siWaveBanner() {
  siBannerShow(`Wave ${SIG.g.wave} — clear them all`);
}

/* Phase 1: LEVEL COMPLETE notification. Reuses the wave-banner element
 * (same styling, same non-blocking behaviour) so the visual design is
 * untouched; it stays up for the whole transition beat and is hidden by
 * siLevelHide() when the next level actually begins. Generation-guarded
 * so no pending timer can flash or clear it mid-beat. */
function siLevelDone() {
  const el = $('#siWaveBanner'); if (!el) return;
  SIG.bannerGen = (SIG.bannerGen || 0) + 1;   // cancel any pending hide
  el.textContent = 'LEVEL COMPLETE';
  el.classList.add('show', 'level');
}
function siLevelHide() {
  const el = $('#siWaveBanner'); if (!el) return;
  SIG.bannerGen = (SIG.bannerGen || 0) + 1;   // cancel any pending hide
  el.classList.remove('show', 'level');
}

/* Phase 2: POWER-UP ACQUIRED notification. Reuses the same banner element
 * and guard system as LEVEL COMPLETE. Shown during the between-levels beat
 * alongside it (stacked below via CSS), and hidden when the level starts. */
function siPowerBanner(kind) {
  const label = SI.POWERS[kind] ? SI.POWERS[kind].label : String(kind);
  const el = $('#siPowerBanner'); if (!el) return;
  SIG.pbannerGen = (SIG.pbannerGen || 0) + 1;
  el.textContent = `POWER-UP ACQUIRED: ${label}!`;
  el.classList.add('show');
  // Auto-hide shortly after the next level begins so both banners are
  // visible during the beat but the field is clear once play resumes.
  const myGen = SIG.pbannerGen, myEpoch = SIG.epoch;
  window.setTimeout(() => {
    if (myEpoch !== SIG.epoch || myGen !== SIG.pbannerGen) return;
    el.classList.remove('show');
  }, 2600);
}
function siPowerBannerHide() {
  const el = $('#siPowerBanner'); if (!el) return;
  SIG.pbannerGen = (SIG.pbannerGen || 0) + 1;
  el.classList.remove('show');
}

/* Phase 2: record an expiry so the chip strip renders one final
 * "ghost" chip with the fade-out animation (the strip is rebuilt each
 * frame, so the class must be part of the render, not added after it). */
function siPowerExpired(kind) {
  SIG.powerFlash = SIG.powerFlash || {};
  SIG.powerFlash[kind] = performance.now();
}

/* ---- actions --------------------------------------------------------- */
function siStart() {
  const g = SIG.g;
  // Phase 3/4: Start/Resume is a user gesture — unlock audio here, then
  // let siMusicEnsure pick the right track for the new state.
  siAudioUnlock();
  // Phase 1: resume into whichever mode was paused — the between-levels
  // beat continues its automatic countdown instead of skipping it.
  if (g.state === 'paused' && SIG.pausedFrom === 'between') {
    g.state = 'between';
    siOverlay(null);
    siPauseMusic(false);            // Phase 3: restore music level
    return;
  }
  if (g.state === 'ready' || g.state === 'paused') { g.state = 'playing'; siOverlay(null); siPauseMusic(false); }
}
function siPause() {
  const g = SIG.g;
  // Phase 1: pausing during the between-levels beat freezes the automatic
  // countdown too; resuming continues it where it left off.
  if (g.state === 'playing' || g.state === 'between') {
    SIG.pausedFrom = g.state;         // remember which mode to resume into
    g.state = 'paused';
    siOverlay('paused');
    siPauseMusic(true);               // Phase 3: duck the music while paused
  }
}
function siTogglePause() { SIG.g.state === 'paused' ? siStart() : siPause(); }

function siRestart() {
  const keepHigh = SIG.g ? SIG.g.highScore : 0;
  SIG.g = SI.newGame();
  SIG.g.highScore = keepHigh;
  SIG.g.state = 'playing';
  SIG.pausedFrom = null;              // Phase 1: no stale resume mode
  siOverlay(null);
  siLevelHide();                      // drop any LEVEL COMPLETE banner
  siPowerBannerHide();                // Phase 2: drop any POWER-UP banner
  // Phase 3/4: restart is a user gesture; audio unlocks and the music
  // returns to the fresh gameplay track at intensity tier 1 (wave 1).
  siAudioUnlock();
  siPauseMusic(false);
  if (SIA.jingleUntil) { SIA.jingleUntil = 0; SIA.prevTrack = null; }
  siMusicIntensity(1);
  siMusicEnsure();
  // The frame loop announces the wave via SI_HUD_CACHE.waveShown; clear
  // the cache entry so a restart to wave 1 re-announces it.
  delete SI_HUD_CACHE.waveShown;
  delete SI_HUD_CACHE.powerSig;       // Phase 2: force chips to re-render
  SIG.powerFlash = null;              // Phase 2: drop pending expiry ghosts
}

/* ---- teardown -------------------------------------------------------- */
/** Remove every listener, cancel the loop, clear held input. Safe to call
 *  repeatedly; re-entering the route builds fresh state. */
function siTeardown() {
  SIG.epoch++;                    // invalidates the running loop + timers
  if (SIG.raf) { cancelAnimationFrame(SIG.raf); SIG.raf = 0; }
  for (const off of SIG.cleanup) { try { off(); } catch { /* already gone */ } }
  SIG.cleanup = [];
  SIG.keys = Object.create(null);
  SIG.touch = { left: false, right: false, fire: false };
  SIG.ctx = null; SIG.g = null; SIG.stars = null;
  SIG.pausedFrom = null;              // Phase 1: no stale resume mode
  SIG.powerFlash = null;              // Phase 2: no stale expiry ghosts
  for (const k of Object.keys(SI_HUD_CACHE)) delete SI_HUD_CACHE[k];  // Leaving the route must never leave audio playing or a fullscreen
  // layout applied to the dashboard shell.
  siAudioDispose();
  ftExitFullscreenQuiet();
  document.body.classList.remove('si-fs');
  ftSessionRestore();               // restore dashboard chrome
}


/* ---- Fun Time view ------------------------------------------------
 * Game state, scoped to this route. Kept in a single object so a route
 * change can tear everything down in one place (see the teardown guard).
 * `epoch` is the key correctness device: every asynchronous continuation
 * (the AI's delayed reply) captures the epoch at schedule time and refuses
 * to run if the game was reset meanwhile — a stale timer can never mutate
 * a freshly reset board. */
const FT = {
  board: Array(9).fill(''),
  turn: 'X',                 // whose move is next ('X' human, 'O' AI)
  finished: null,            // T3.result(...) once terminal
  scores: { you: 0, ai: 0, draw: 0 },
  difficulty: 'impossible',
  thinking: false,           // an AI reply is scheduled/in flight
  epoch: 0,                  // bumped on every reset/teardown
  aiDelayMs: 380,            // small beat so the AI feels like a player
  fsCleanup: [],             // T3 fullscreen listener removals
};

function ftResetBoard(keepScores = true) {
  FT.epoch++;                       // invalidates any pending AI callback
  FT.board = Array(9).fill('');
  FT.turn = 'X';
  FT.finished = null;
  FT.thinking = false;
  if (!keepScores) FT.scores = { you: 0, ai: 0, draw: 0 };
}

/** Apply the human's move and schedule the AI reply. Returns false when the
 *  click was not a legal human move (occupied cell, not their turn, game
 *  over, or an AI reply already pending) — rapid clicks are simply ignored,
 *  never queued. */
function ftHumanMove(i) {
  if (FT.turn !== 'X' || FT.finished) return false;
  if (i < 0 || i > 8 || FT.board[i] !== '') return false;
  FT.board[i] = 'X';

  FT.finished = T3.result(FT.board);
  if (FT.finished) { ftSettle(FT.finished); return true; }

  FT.turn = 'O';
  FT.thinking = true;
  const myEpoch = FT.epoch;
  window.setTimeout(() => {
    // Guard: the game may have been reset (or the route left) while this
    // timer was pending. Only the newest epoch may act.
    if (myEpoch !== FT.epoch) return;
    ftAiReply();
    // The AI's reply changed state; reflect it. (A reset during the reply
    // bumps the epoch, so this render can only ever draw current state.)
    ftRenderBoard();
    ftRenderStatus();
  }, FT.aiDelayMs);
  return true;
}

/** The AI's single reply for one human action. Exactly one call per human
 *  move: `thinking` is cleared here and never re-entered. */
function ftAiReply() {
  if (FT.turn !== 'O' || FT.finished) { FT.thinking = false; return; }
  const m = T3.pick(FT.board, FT.difficulty);
  FT.thinking = false;
  if (m == null) return;             // terminal/no move: nothing to do
  if (FT.board[m] !== '') return;    // paranoia: never touch an occupied cell
  FT.board[m] = 'O';

  FT.finished = T3.result(FT.board);
  if (FT.finished) { ftSettle(FT.finished); return; }
  FT.turn = 'X';
}

/** Record a completed round exactly once and refresh the scoreboard. */
function ftSettle(result) {
  FT.turn = null;
  if (result.kind === 'win') FT.scores.you++;
  else if (result.kind === 'loss') FT.scores.ai++;
  else FT.scores.draw++;
  ftRenderScores();
}

/* Result banner text for a finished round. */
function ftBanner(r) {
  if (!r) return '';
  if (r.kind === 'win') return 'You win!';
  if (r.kind === 'loss') return 'The AI wins.';
  return "It's a draw.";
}

/* Difficulty labels used by both the selector and the hint line. */
const FT_DIFFS = [
  ['easy', 'Easy'],
  ['medium', 'Medium'],
  ['impossible', 'Impossible'],
];

/* ---- Fun Time routing ------------------------------------------------
 * Fun Time is one dashboard route with three sub-views, addressed by hash:
 *   #funtime       -> the game-selection screen (two cards)
 *   #funtime/t3    -> Tic-Tac-Toe AI, dedicated view
 *   #funtime/si    -> Space Invaders, dedicated view
 *   #funtime/ht    -> Hack the Terminal, dedicated view
 * Sub-hashes ride the SPA's existing hashchange router, so browser
 * back/forward moves between the selector and a game for free. Only one
 * view is ever mounted at a time; leaving one tears its listeners down.
 */
/** Pipeline-workspace sub-routes:
 *   #pretrain                -> stage view (default)
 *   #pretrain/architecture   -> pretraining architecture canvas
 *   #sft                     -> run history (default)
 *   #sft/workflow            -> SFT workflow canvas (documented design)
 *   #sft/architecture        -> SFT architecture canvas (transcribed)
 *   #sft/board               -> EKA SFT data-pipeline board (iframe)
 *   #sft/code                -> code explorer over the real Python files
 */
function paSubRoute() {
  const h = (location.hash || '').replace(/^#/, '');
  if (h.startsWith('pretrain/architecture')) return 'architecture';
  if (h.startsWith('pretrain/report')) return 'report';
  if (h.startsWith('sft/workflow')) return 'workflow';
  if (h.startsWith('sft/architecture')) return 'sft-architecture';
  if (h.startsWith('sft/board')) return 'sft-board';
  if (h.startsWith('sft/code')) return 'sft-code';
  return 'stages';
}

function ftSubRoute() {
  const h = (location.hash || '').replace(/^#/, '');
  if (h.startsWith('funtime/t3')) return 't3';
  if (h.startsWith('funtime/si')) return 'si';
  if (h.startsWith('funtime/ht')) return 'ht';
  if (h.startsWith('funtime/cr')) return 'cr';
  if (h.startsWith('funtime/ch')) return 'ch';
  if (h.startsWith('funtime/tm')) return 'tm';
  if (h.startsWith('funtime/so')) return 'so';
  if (h.startsWith('funtime/fn')) return 'fn';
  if (h.startsWith('funtime/lu')) return 'lu';
  if (h.startsWith('funtime/sn')) return 'sn';
  return 'select';
}

async function renderFunTime() {
  $('#pageTitle').textContent = 'Fun Time';
  $('#pageSub').textContent = 'take a break · play a game';

  // Any previous game must be fully torn down before mounting a view —
  // the selector shows no gameplay, and a new game gets fresh listeners.
  siTeardown();
  htTeardown();
  crTeardown();
  if (typeof chTeardown === 'function') chTeardown();
  if (typeof tmTeardown === 'function') tmTeardown();
  if (typeof soTeardown === 'function') soTeardown();
  if (typeof fnTeardown === 'function') fnTeardown();
  if (typeof luTeardown === 'function') luTeardown();
  if (typeof snTeardown === 'function') snTeardown();

  const sub = ftSubRoute();
  if (sub === 't3') return renderT3View();
  if (sub === 'si') return renderSiView();
  if (sub === 'ht') return renderHtView();
  if (sub === 'cr') return renderCrView();
  if (sub === 'ch' && typeof renderChView === 'function') return renderChView();
  if (sub === 'tm' && typeof renderTmView === 'function') return renderTmView();
  if (sub === 'so' && typeof renderSoView === 'function') return renderSoView();
  if (sub === 'fn' && typeof renderFnView === 'function') return renderFnView();
  if (sub === 'lu' && typeof renderLuView === 'function') return renderLuView();
  if (sub === 'sn' && typeof renderSnView === 'function') return renderSnView();
  return renderFtSelect();
}

/* ============================================================
   SHARED GAME FULLSCREEN SYSTEM (FTFS)
   One convention for all four Fun Time games. Each game keeps its own
   engine, but launch/exit/fullscreen behaviour is identical everywhere:

     • Fullscreen is REAL browser fullscreen (Fullscreen API on the game
       panel). CSS-only "fake fullscreen" is never used or implied.
     • Entry always originates from a user gesture (the Fullscreen button,
       the F key, or the launch overlay's Enter Fullscreen button) because
       browsers reject requests made outside one.
     • Escape lets the BROWSER exit fullscreen natively; we never swallow
       it. Our keydown handler only mirrors the state afterwards.
     • Exiting fullscreen returns to the inline game view with the normal
       dashboard layout restored — never a stuck class or frozen loop.
     • A visible Back to Games control exists in every game view.

   Per-game compatibility wrappers (siIsFullscreen, crFsSupported, …) keep
   the established public names so existing callers and tests are stable.
   ============================================================ */
const FTFS = {
  active: null,          // 'si' | 't3' | 'ht' | 'cr' — which game is fullscreened
  denyNoted: false,      // show the "play inline" notice only once per session
};

/** Genuine fullscreen element, cross-engine. */
function ftFsElement() {
  return document.fullscreenElement
    || document.webkitFullscreenElement || null;
}
/** Is the real Fullscreen API available at all? */
function ftFsSupported() {
  const el = document.documentElement;
  return !!(el.requestFullscreen || el.webkitRequestFullscreen);
}
function ftIsFullscreen() { return !!ftFsElement(); }

/**
 * Enter/leave genuine browser fullscreen for a game panel.
 * `game` is the short id ('si'|'t3'|'ht'|'cr'); `cls` is the body class the
 * game's CSS expects ('si-fs', 't3-fs', 'ht-fs', 'cr-fs').
 * Returns true when the desired state was reached.
 */
async function ftToggleFullscreen(game, cls, noticeId) {
  const panel = $('#' + game + 'Panel');
  if (!panel) return false;

  // ---- leaving ----
  if (ftIsFullscreen()) {
    try {
      if (document.exitFullscreen) await document.exitFullscreen();
      else if (document.webkitExitFullscreen) document.webkitExitFullscreen();
    } catch { /* browser refused; reflect() will reconcile */ }
    return !ftIsFullscreen();
  }

  // ---- entering ----
  const req = panel.requestFullscreen || panel.webkitRequestFullscreen;
  if (!req) {
    ftFsNotice(noticeId, 'Fullscreen is not supported here — playing inline.', game);
    return false;
  }
  try {
    // The caller must be inside a user gesture (click / keydown).
    await req.call(panel);
    return ftIsFullscreen();
  } catch {
    // Denied (iframe policy, permissions, or lost gesture). Never retry in
    // a loop — tell the user once and keep the game fully playable inline.
    ftFsNotice(noticeId, 'Could not enter fullscreen — playing inline.', game);
    FTFS.denyNoted = true;
    return false;
  }
}

/** Non-blocking notice used for fullscreen feedback. Epoch-guarded so a
 *  stale timer can never touch a different game's DOM. */
function ftFsNotice(noticeId, msg, game) {
  const el = noticeId ? $(noticeId) : null;
  if (!el) return;
  el.textContent = msg;
  el.classList.add('show');
  const epoch = FTFS.epochOf(game);
  setTimeout(() => { if (epoch === FTFS.epochOf(game)) el.classList.remove('show'); }, 2600);
}
FTFS.epochOf = game => {
  const g = window[({ si: 'SIG', t3: 'T3G', ht: 'HT', cr: 'CRG', ch: 'CHG', tm: 'TMG', so: 'SOG', fn: 'FNG', lu: 'LUG', sn: 'SNG' })[game]];
  return g ? g.epoch : -1;
};

/**
 * Reflect genuine fullscreen state onto the body class + button label.
 * Called from the single fullscreenchange listener each game registers via
 * its own cleanup list. `cls` is that game's body class; all other games'
 * classes are removed defensively so no stale class survives a fast switch.
 */
function ftFsReflect(game, cls, btnId) {
  const on = ftIsFullscreen();
  FTFS.active = on ? game : null;
  // Remove every game's fs class, then apply only the current one. This is
  // what guarantees no stuck class after CR→SI style direct switches.
  for (const c of ['si-fs', 't3-fs', 'ht-fs', 'cr-fs', 'ch-fs', 'tm-fs', 'so-fs', 'fn-fs', 'lu-fs', 'sn-fs']) {
    if (c === cls && on) document.body.classList.add(c);
    else document.body.classList.remove(c);
  }
  const btn = btnId ? $(btnId) : null;
  if (btn) {
    btn.setAttribute('aria-pressed', String(on));
    btn.textContent = on ? 'Exit Fullscreen' : 'Fullscreen';
  }
  return on;
}

/** Standard Escape handling for a game's keydown listener. Returns true if
 *  the key was consumed.
 *
 *  Real browsers exit fullscreen on Escape natively; some embedded engines
 *  and headless runners do not. We therefore ALSO request the exit
 *  explicitly — calling document.exitFullscreen() is idempotent (it either
 *  performs the same transition the browser already started, or resolves
 *  immediately when nothing is fullscreen), so this can never block or
 *  double-exit. This is what makes Escape reliable everywhere without
 *  intercepting it away from the browser. */
function ftEscapeKey(e) {
  if (e.key !== 'Escape') return false;
  if (!ftIsFullscreen()) return false;
  e.preventDefault();
  ftExitFullscreenQuiet();
  return true;
}

/** Register the shared fullscreenchange plumbing for a game. `off` is the
 *  game's own listener-remover registrar (t3On/siOn/htOn/crOn), `onResize`
 *  refits that game's canvas/board. */
function ftBindFullscreen(off, game, cls, btnId, noticeId, onResize) {
  off($('#' + game + 'FsBtn'), 'click', async () => {
    await ftToggleFullscreen(game, cls, noticeId);
  });
  const reflect = () => { ftFsReflect(game, cls, btnId); if (onResize) onResize(); };
  off(document, 'fullscreenchange', reflect);
  off(document, 'webkitfullscreenchange', reflect);
  // Hide the control entirely when real fullscreen cannot happen; the game
  // stays fully playable in its dedicated inline view.
  if (!ftFsSupported()) {
    const b = $('#' + game + 'FsBtn');
    if (b) b.style.display = 'none';
  }
}

/** Leave fullscreen quietly during teardown (never throws, never races). */
function ftExitFullscreenQuiet() {
  if (!ftIsFullscreen()) return;
  try {
    if (document.exitFullscreen) {
      const r = document.exitFullscreen();
      if (r && r.catch) r.catch(() => {});
    } else if (document.webkitExitFullscreen) document.webkitExitFullscreen();
  } catch { /* already leaving */ }
}

/**
 * The launch row shown on a game's ready/launch overlay: a prominent
 * "Enter Fullscreen" button plus a one-line hint. This is the reliable,
 * single-click gesture path into real browser fullscreen.
 *
 * `off`  – the game's listener registrar (siOn/htOn/crOn).
 * `game` – 'si' | 't3' | 'ht' | 'cr'; used for the panel id and body class.
 * `mountId` – element inside the overlay to append the row to.
 * Returns nothing; safe to call again after re-render (it replaces content).
 */
function ftLaunchRow(off, game, mountId) {
  const mount = $(mountId);
  if (!mount) return;
  const cls = game + '-fs';
  mount.innerHTML = `
    <div class="ft-ov-launch">
      <button type="button" class="ft-ov-fs" id="${game}OvFsBtn" aria-pressed="false">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"
             stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M16 3h3a2 2 0 0 1 2 2v3"/>
          <path d="M8 21H5a2 2 0 0 1-2-2v-3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/>
        </svg>
        <span id="${game}OvFsLbl">Enter Fullscreen</span>
      </button>
      <div class="ft-ov-hint">or press F · Esc leaves fullscreen · Back to Games returns here</div>
    </div>`;

  // Keep the button label in sync with genuine fullscreen state.
  const paint = () => {
    const btn = $('#' + game + 'OvFsBtn');
    const lbl = $('#' + game + 'OvFsLbl');
    const on = ftIsFullscreen();
    if (btn) btn.setAttribute('aria-pressed', String(on));
    if (lbl) lbl.textContent = on ? 'Leave Fullscreen' : 'Enter Fullscreen';
  };
  off($('#' + game + 'OvFsBtn'), 'click', async () => {
    await ftToggleFullscreen(game, cls, null);
    paint();
  });
  off(document, 'fullscreenchange', paint);
  off(document, 'webkitfullscreenchange', paint);
  if (!ftFsSupported()) {
    const b = $('#' + game + 'OvFsBtn');
    if (b) {
      b.style.display = 'none';
      const h = mount.querySelector('.ft-ov-hint');
      if (h) h.textContent = 'fullscreen is not supported in this browser — playing inline';
    }
  }
  paint();
}

/* ============================================================
   SHARED GAME-SESSION CONTROLLER (FTSESS)
   One lifecycle for ALL FIVE Fun Time games (t3/si/ht/cr/ch):

     card click → request native fullscreen IN the gesture → mount
       → if native denied/unavailable: automatic CSS immersive fallback
     ESC / browser-exit / Back-to-Games → leave game → #funtime

   Differences from the older FTFS helpers above (which remain for the
   in-game Fullscreen toggle buttons):
     • Launch is AUTOMATIC from the card click — no extra button.
     • Esc leaves the GAME, not just fullscreen.
     • Dashboard chrome (sidebar/topbar) is hidden during play and
       restored afterwards via a single scoped body class.
     • A `leaving` guard makes keydown + fullscreenchange agree on
       exactly one navigation.

   Chess keeps its own state machine (CHG.fs) for its layout specifics,
   but routes launch/Esc through this controller so all five games feel
   identical from the user's point of view.
   ============================================================ */
const FTSESS = {
  pending: null,      // game id whose card was just clicked ('si'|'t3'|...)
  leaving: false,     // one navigation per exit, no matter which handler fires
  prevNative: false,  // was native fullscreen on at the previous event
  sessions: 0,        // mounts this page session (diagnostics)
};

/** Is the dashboard currently showing a full game view (not the selector)? */
function ftSessionActive() {
  const sub = ftSubRoute();
  return sub === 't3' || sub === 'si' || sub === 'ht' || sub === 'cr' || sub === 'ch' || sub === 'tm' || sub === 'so' || sub === 'fn' || sub === 'lu' || sub === 'sn';
}

/** Request native fullscreen synchronously inside the card's click gesture.
 *  Must run BEFORE any await/timer/navigation. Returns nothing; the result
 *  is reconciled by ftSessionReflect() after the view mounts. */
function ftSessionLaunch(game) {
  FTSESS.pending = game;
  FTSESS.leaving = false;
  const el = document.documentElement;
  const req = el.requestFullscreen || el.webkitRequestFullscreen;
  if (!req) return;                    // CSS fallback engages after mount
  try {
    const r = req.call(el);
    if (r && r.catch) r.catch(() => { /* denied → CSS fallback after mount */ });
  } catch { /* denied → CSS fallback after mount */ }
}

/** Reflect "a game is playing" onto the document: hide dashboard chrome.
 *  Called by every game view after mount. `immersive` additionally marks
 *  the CSS fallback (native fullscreen unavailable/denied). */
function ftSessionReflect(immersive) {
  const active = ftSessionActive();
  document.body.classList.toggle('ft-playing', active);
  document.body.classList.toggle('ft-immersive', active && !!immersive);
  if (!active) { FTSESS.leaving = false; FTSESS.prevNative = false; }
}

/** Leave the active game and return to the Fun Time selector. Exactly-once
 *  navigation guarded against racing Escape-keydown and fullscreenchange. */
function ftSessionLeave() {
  if (!ftSessionActive()) return;
  if (FTSESS.leaving) return;          // already navigating
  FTSESS.leaving = true;
  // Ask the browser to leave native fullscreen too (idempotent; some
  // engines don't process Esc natively).
  ftExitFullscreenQuiet();
  if (location.hash.startsWith('#funtime/')) location.hash = '#funtime';
}

/** Wire the shared Esc / fullscreenchange behaviour for a game view.
 *  `off` is that game's listener registrar; `onResize` refits its canvas.
 *  Games keep their own keydown handlers for gameplay keys — this only
 *  owns Escape and the fullscreen transition. */
function ftSessionBind(off, onResize) {
  // Esc anywhere while a game view is active returns to the selector.
  off(window, 'keydown', e => {
    if (e.key !== 'Escape') return;
    if (!ftSessionActive()) return;            // not our view — don't touch it
    if (e.target && (e.target.tagName === 'INPUT'
                     || e.target.tagName === 'TEXTAREA'
                     || e.target.isContentEditable)) return;
    ftSessionLeave();
  });
  // Native fullscreen ended (Esc or a browser control): if we were in it,
  // that means "leave the game", never a broken intermediate state.
  const changed = () => {
    const native = ftIsFullscreen();
    if (native) { FTSESS.prevNative = true; }
    else if (FTSESS.prevNative) {
      FTSESS.prevNative = false;
      ftSessionLeave();                        // guarded; runs once
    }
    /* Phase 5.6: keep ft-immersive honest. A card-click launch may mount
       the view BEFORE its requestFullscreen() promise resolves — mount
       sees no fullscreen element and engages the CSS-immersive fallback.
       When native fullscreen then lands, that fallback class must be
       dropped: the two apply the same layout, but reporting "immersive"
       while genuinely full screen confuses every state check (and the
       Fullscreen button). ftSessionReflect(false) recomputes both
       classes from the real session state. */
    if (native && document.body.classList.contains('ft-immersive')) {
      ftSessionReflect(false);
    }
    if (onResize) onResize();
  };
  off(document, 'fullscreenchange', changed);
  off(document, 'webkitfullscreenchange', changed);
}

/** Called by renderFunTime when a card was just clicked: after the view
 *  mounted, decide native vs CSS-immersive based on what actually happened
 *  during the gesture. Never claims native when it isn't. */
function ftSessionAfterMount(game, cls) {
  FTSESS.sessions++;
  const native = ftIsFullscreen();
  const wanted = FTSESS.pending === game;
  FTSESS.pending = null;
  if (native) {
    // Real fullscreen achieved. Apply the game's fs class for its layout.
    document.body.classList.add(cls);
    FTSESS.prevNative = true;
  } else if (wanted) {
    // Card-click launch but native did not stick (denied, iframe policy,
    // unsupported). Automatic CSS immersive fallback — same layout class,
    // honestly reported as NOT native fullscreen.
    document.body.classList.add(cls);
  }
  ftSessionReflect(!native && wanted);
}

/** Restore the normal dashboard layout. Called from each teardown. */
function ftSessionRestore(cls) {
  if (cls) document.body.classList.remove(cls);
  ftSessionReflect(false);
}

/* Suit glyphs for the Solitaire selector-card preview (kept next to the
   card markup that uses them). */
const SO_ART_HEART = '\u2665';
const SO_ART_SPADE = '\u2660';

/* ---- Game-selection screen ------------------------------------------ */
function renderFtSelect() {
  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <div class="ft-hero">
      <div>
        <h1 class="ft-h1">Fun Time</h1>
        <div class="ft-tag">Take a break. Play a game.</div>
      </div>
      <a class="ft-back" href="#overview" title="Back to the dashboard">← back to dashboard</a>
    </div>

    <div class="ft-select" id="ftSelect">
      <!-- Card 1: Tic-Tac-Toe AI -->
      <a class="ft-card" id="ftCardT3" href="#funtime/t3" aria-label="Play Tic-Tac-Toe AI">
        <div class="ft-card-art t3" aria-hidden="true">
          <span class="ft-card-grid">
            <span></span><span class="x">✕</span><span></span>
            <span></span><span></span><span class="o">◯</span>
            <span class="x">✕</span><span></span><span></span>
          </span>
        </div>
        <div class="ft-card-body">
          <div class="ft-card-title">Tic-Tac-Toe AI</div>
          <div class="ft-card-desc">You are X. Beat the machine — or settle for a draw. Three difficulty modes, including a perfect player.</div>
          <span class="ft-play">Play Now <span aria-hidden="true">→</span></span>
        </div>
      </a>

      <!-- Card 2: Space Invaders -->
      <a class="ft-card" id="ftCardSi" href="#funtime/si" aria-label="Play Space Invaders">
        <div class="ft-card-art si" aria-hidden="true">
          <span class="ft-card-invader">
            <svg viewBox="0 0 64 48" fill="currentColor" aria-hidden="true">
              <rect x="24" y="4" width="16" height="6"/>
              <rect x="16" y="10" width="32" height="8"/>
              <rect x="12" y="18" width="40" height="8"/>
              <rect x="8"  y="26" width="8"  height="8"/>
              <rect x="48" y="26" width="8"  height="8"/>
              <rect x="20" y="26" width="24" height="6"/>
              <rect x="24" y="34" width="6" height="8"/>
              <rect x="34" y="34" width="6" height="8"/>
            </svg>
          </span>
        </div>
        <div class="ft-card-body">
          <div class="ft-card-title">Space Invaders</div>
          <div class="ft-card-desc">Defend the line against descending alien waves. Your ship fires automatically — just steer and survive.</div>
          <span class="ft-play">Play Now <span aria-hidden="true">→</span></span>
        </div>
      </a>

      <!-- Card 3: Hack the Terminal -->
      <a class="ft-card" id="ftCardHt" href="#funtime/ht" aria-label="Play Hack the Terminal">
        <div class="ft-card-art ht" aria-hidden="true">
          <span class="ht-card-term">
            <span class="ln"><span class="p">operator@loop</span>:~$ system.connect()</span>
            <span class="ln c">connection established…</span>
            <span class="ln"><span class="p">operator@loop</span>:~$ access.verify</span>
            <span class="ln err" style="color:var(--red)">ACCESS DENIED</span>
            <span class="ln ok">&gt; awaiting operator<span class="cur"></span></span>
          </span>
        </div>
        <div class="ft-card-body">
          <div class="ft-card-title">Hack the Terminal</div>
          <div class="ft-card-desc">Crack four security locks — codes, ciphers and firewalls — before the connection times out.</div>
          <span class="ft-play">Start Game <span aria-hidden="true">→</span></span>
        </div>
      </a>

      <!-- Card 4: Cyber Rush (street racing) -->
      <a class="ft-card" id="ftCardCr" href="#funtime/cr" aria-label="Play Soket Street Racing Cyber Rush">
        <div class="ft-card-art cr" aria-hidden="true">
          <span class="cr-card-scene">
            <span class="cr-card-road"></span>
            <span class="cr-card-lane l"></span>
            <span class="cr-card-lane r"></span>
            <span class="cr-card-car">
              <svg viewBox="0 0 34 20" fill="currentColor" aria-hidden="true">
                <rect x="4" y="7" width="26" height="9" rx="2.5"/>
                <rect x="9" y="3" width="16" height="6" rx="2"/>
                <rect x="5" y="14" width="5" height="3" rx="1"/>
                <rect x="24" y="14" width="5" height="3" rx="1"/>
              </svg>
            </span>
            <span class="cr-card-glow"></span>
          </span>
        </div>
        <div class="ft-card-body">
          <div class="ft-card-title">Cyber Rush</div>
          <div class="ft-card-desc">Soket Street Racing: a neon-night pursuit through traffic. Chase the target score, thread the gaps, burn the nitro.</div>
          <span class="ft-play">Start Game <span aria-hidden="true">→</span></span>
        </div>
      </a>

      <!-- Card 5: Chess -->
      <a class="ft-card" id="ftCardCh" href="#funtime/ch" aria-label="Play Chess">
        <div class="ft-card-art ch" aria-hidden="true">
          <span class="ch-card-knight">&#9822;</span>
          <span class="ch-card-strip"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></span>
        </div>
        <div class="ft-card-body">
          <div class="ft-card-title">Chess</div>
          <div class="ft-card-desc">Full-rules chess — castling, en passant, promotion. Beat a real minimax AI at four strengths, or play a friend locally.</div>
          <span class="ft-play">Play Now <span aria-hidden="true">→</span></span>
        </div>
      </a>

      <!-- Card 6: Timber Man Ultra -->
      <a class="ft-card" id="ftCardTm" href="#funtime/tm" aria-label="Play Timber Man Ultra">
        <div class="ft-card-art tm" aria-hidden="true">
          <span class="tm-card-scene">
            <span class="tm-card-trunk"></span>
            <span class="tm-card-branch l"></span>
            <span class="tm-card-branch r"></span>
            <span class="tm-card-man">
              <svg viewBox="0 0 30 34" fill="none" aria-hidden="true">
                <circle cx="15" cy="7" r="5" fill="#f2c19a"/>
                <path d="M8 4h14l-2-3H10z" fill="#ffb703"/>
                <rect x="9" y="13" width="12" height="13" rx="4" fill="#d64545"/>
                <path d="M11 26l-3 8M19 26l3 8" stroke="#2c3a55" stroke-width="3.4" stroke-linecap="round"/>
                <path d="M20 16l8-6" stroke="#f2c19a" stroke-width="3.4" stroke-linecap="round"/>
                <path d="M26 11l4-3" stroke="#8a5c33" stroke-width="2.6" stroke-linecap="round"/>
                <rect x="28" y="5" width="6" height="8" rx="1.6" fill="#cfd8e3" transform="rotate(24 31 9)"/>
              </svg>
            </span>
          </span>
        </div>
        <div class="ft-card-body">
          <div class="ft-card-title">Timber Man Ultra</div>
          <div class="ft-card-desc">Chop fast, dodge branches, race the clock. Every chop adds time — but the refund shrinks as your score climbs.</div>
          <span class="ft-play">Play Now <span aria-hidden="true">→</span></span>
        </div>
      </a>
      <!-- Card 8: Fruit Ninja -->
      <a class="ft-card" id="ftCardFn" href="#funtime/fn" aria-label="Play Fruit Ninja">
        <div class="ft-card-art fn" aria-hidden="true">
          <span class="fn-card-scene">
            <span class="fn-card-melon"><i></i><i></i><i></i></span>
            <span class="fn-card-slash"></span>
            <span class="fn-card-spark"></span>
          </span>
        </div>
        <div class="ft-card-body">
          <div class="ft-card-title">Fruit Ninja</div>
          <div class="ft-card-desc">Drag to slice flying fruit into juicy halves. Chain combos, dodge bombs, chase your best in Classic and Arcade.</div>
          <span class="ft-play">Start Game <span aria-hidden="true">→</span></span>
        </div>
      </a>

      <!-- Card 7: Solitaire (Classic Klondike) -->
      <a class="ft-card" id="ftCardSo" href="#funtime/so" aria-label="Play Solitaire Classic Klondike">
        <div class="ft-card-art so" aria-hidden="true">
          <span class="so-card-art">
            <span class="so-art-back"><span class="so-art-pattern"></span></span>
            <span class="so-art-front red">
              <span class="so-art-idx">A</span>
              <span class="so-art-pip">${SO_ART_HEART}</span>
            </span>
            <span class="so-art-front black">
              <span class="so-art-idx">K</span>
              <span class="so-art-pip">${SO_ART_SPADE}</span>
            </span>
          </span>
        </div>
        <div class="ft-card-body">
          <div class="ft-card-title">Solitaire</div>
          <div class="ft-card-desc">Classic Klondike, Draw 1. Build the four foundations from Ace to King — undo, hints and a running score included.</div>
          <span class="ft-play">Play Now <span aria-hidden="true">→</span></span>
        </div>
      </a>

      <!-- Card 8: Ludo (Classic 4-Player) -->
      <a class="ft-card" id="ftCardLu" href="#funtime/lu" aria-label="Play Ludo classic four-player">
        <div class="ft-card-art lu" aria-hidden="true">
          <span class="lu-card-scene">
            <span class="lu-card-board"></span>
            <span class="lu-card-pawn r"></span>
            <span class="lu-card-pawn g"></span>
            <span class="lu-card-pawn y"></span>
            <span class="lu-card-pawn b"></span>
          </span>
        </div>
        <div class="ft-card-body">
          <div class="ft-card-title">Ludo</div>
          <div class="ft-card-desc">Classic four-player race. Roll a six to release your tokens, capture rivals, dodge stars and bring all four home.</div>
          <span class="ft-play">Play Now <span aria-hidden="true">→</span></span>
        </div>
      </a>

      <!-- Card 9: Snakes & Ladders -->
      <a class="ft-card" id="ftCardSn" href="#funtime/sn" aria-label="Play Snakes and Ladders">
        <div class="ft-card-art sn" aria-hidden="true">
          <span class="sn-card-scene">
            <span class="sn-card-grid">
              <i></i><i></i><i></i><i></i><i></i>
              <i></i><i></i><i></i><i></i><i></i>
              <i></i><i></i><i></i><i></i><i></i>
              <i></i><i></i><i></i><i></i><i></i>
            </span>
            <span class="sn-card-ladder" aria-hidden="true"></span>
            <span class="sn-card-snake" aria-hidden="true"></span>
            <span class="sn-card-pawn r" aria-hidden="true"></span>
            <span class="sn-card-pawn g" aria-hidden="true"></span>
          </span>
        </div>
        <div class="ft-card-body">
          <div class="ft-card-title">Snakes &amp; Ladders</div>
          <div class="ft-card-desc">The classic 100-square race. Climb ladders, dodge snakes, land exactly on 100. Play friends or the computer — up to four players.</div>
          <span class="ft-play">Play Now <span aria-hidden="true">→</span></span>
        </div>
      </a>

    </div>
  </div></div>`;

  /* Card-click launch hook for ALL FIVE games: request native fullscreen
     INSIDE the click gesture (browsers reject requests outside one), then
     let the hash navigation mount the game. FTSESS reconciles afterwards —
     if native was denied, the CSS immersive fallback engages automatically.
     Chess routes through its own chTryLaunchFullscreen() so its state
     machine stays the source of truth for its layout. */
  const CARD_GAME = { ftCardT3: 't3', ftCardSi: 'si', ftCardHt: 'ht', ftCardCr: 'cr', ftCardTm: 'tm', ftCardSo: 'so', ftCardFn: 'fn', ftCardLu: 'lu', ftCardSn: 'sn' };
  for (const [cardId, game] of Object.entries(CARD_GAME)) {
    const card = document.getElementById(cardId);
    if (card) card.addEventListener('click', () => ftSessionLaunch(game));
  }
  const chCard = document.getElementById('ftCardCh');
  if (chCard) {
    chCard.addEventListener('click', () => {
      if (typeof chTryLaunchFullscreen === 'function') {
        chTryLaunchFullscreen();   // async; gesture-qualified call, result syncs via chSyncFsState
      }
    });
  }
}

/* ---- Dedicated Tic-Tac-Toe view ------------------------------------- */
function renderT3View() {
  // Fresh visit: start a clean round, keeping any scores from earlier in
  // this page session (Play Again semantics, not New Game).
  ftResetBoard(true);

  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <div class="ft-game-top">
      <a class="ft-back" href="#funtime" title="Back to the games">← back to games</a>
      <div class="ft-game-title">Tic-Tac-Toe AI</div>
    </div>

    <section class="ft-panel ft-game t3-wide" id="t3Panel" aria-label="Tic-Tac-Toe AI">
      <header class="ft-panel-h">
        <div class="ws-ico ft-ico">${ICONS.funtime}</div>
        <div class="ft-panel-t">
          <div class="ft-name">Tic-Tac-Toe AI</div>
          <div class="ft-blurb">You are X. Beat the machine — or settle for a draw.</div>
        </div>
      </header>

      <div class="ft-toolbar" role="group" aria-label="Difficulty">
        ${FT_DIFFS.map(([v, label]) => `
          <button type="button" class="ft-diff${FT.difficulty === v ? ' on' : ''}"
            data-diff="${v}" aria-pressed="${FT.difficulty === v}">${label}</button>`).join('')}
      </div>

      <div class="ft-status" id="t3Status" role="status" aria-live="polite"></div>

      <div class="t3-board-wrap">
        <div class="t3-board" id="t3Board" role="grid" aria-label="Tic-Tac-Toe board"></div>
      </div>

      <div id="t3OvLaunch" hidden></div>

      <div class="ft-actions">
        <button type="button" class="ft-btn" id="t3New">New Game</button>
        <button type="button" class="ft-btn primary" id="t3Again">Play Again</button>
        <button type="button" class="ft-btn" id="t3FsBtn" aria-pressed="false">Fullscreen</button>
        <button type="button" class="ft-btn si-exit" id="t3Exit">Back to Games</button>
      </div>

      <div class="ft-scores" id="t3Scores" aria-label="Scoreboard"></div>
    </section>
  </div></div>`;

  ftRenderBoard();
  ftRenderScores();
  ftRenderStatus();

  // Fullscreen for Tic-Tac-Toe: the shared FTFS system (same real
  // Fullscreen-API convention as every other game). Listeners live on
  // elements replaced by the next render and are removed by t3Teardown.
  const t3On = (target, ev, fn) => {
    target.addEventListener(ev, fn);
    FT.fsCleanup.push(() => target.removeEventListener(ev, fn));
  };
  // Drop any previous T3 fullscreen bindings before adding new ones.
  for (const off of FT.fsCleanup.splice(0)) { try { off(); } catch { /* gone */ } }
  ftBindFullscreen(t3On, 't3', 't3-fs', '#t3FsBtn', null,
                   () => ftRenderBoard());
  // Shared session lifecycle: automatic fullscreen from the card click,
  // Esc → back to Fun Time, chrome hidden while playing.
  ftSessionAfterMount('t3', 't3-fs');
  ftSessionBind(t3On, () => ftRenderBoard());
  // Escape: the browser exits fullscreen natively; we only mirror state.
  t3On(window, 'keydown', e => { ftEscapeKey(e); });
  // Leaving the route must restore the normal layout.
  t3On(window, 'hashchange', () => {
    if (location.hash !== '#funtime/t3') {
      document.body.classList.remove('t3-fs');
      ftExitFullscreenQuiet();
      ftSessionRestore();           // restore dashboard chrome
      for (const off of FT.fsCleanup.splice(0)) { try { off(); } catch { /* gone */ } }
    }
  }, { once: true });

  // Difficulty selector: takes effect from the NEXT AI move onward.
  document.querySelectorAll('.ft-diff').forEach(btn => {
    btn.addEventListener('click', () => {
      FT.difficulty = btn.dataset.diff;
      document.querySelectorAll('.ft-diff').forEach(b => {
        const on = b.dataset.diff === FT.difficulty;
        b.classList.toggle('on', on);
        b.setAttribute('aria-pressed', String(on));
      });
      ftRenderStatus();
    });
  });

  // New Game: board AND scoreboard (the button says so via its hint).
  $('#t3New').addEventListener('click', () => {
    ftResetBoard(false);            // explicit reset of scores too
    ftRenderBoard(); ftRenderScores(); ftRenderStatus();
  });

  // Play Again: new round, scores preserved.
  $('#t3Again').addEventListener('click', () => {
    ftResetBoard(true);
    ftRenderBoard(); ftRenderScores(); ftRenderStatus();
  });

  // Back to Games: navigate via the normal hash router so browser Back
  // returns here. Teardown happens in renderFunTime.
  t3On($('#t3Exit'), 'click', () => { location.hash = '#funtime'; });
}

/* ---- Dedicated Space Invaders view ---------------------------------- */
function renderSiView() {
  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <div class="ft-game-top">
      <a class="ft-back" href="#funtime" title="Back to the games">← back to games</a>
      <div class="ft-game-title">Space Invaders</div>
    </div>

    <section class="ft-panel ft-game si-wide" id="siPanel" aria-label="Space Invaders game">
      <header class="ft-panel-h">
        <div class="ws-ico ft-ico">${ICONS.invader}</div>
        <div class="ft-panel-t">
          <div class="ft-name">Space Invaders</div>
          <div class="ft-blurb">Defend the line. Your ship fires automatically — steer to survive.</div>
        </div>
      </header>

      <div class="si-hud" role="status" aria-live="off">
        <span class="si-stat"><span class="si-k">score</span><span class="si-v" id="siScore">0</span></span>
        <span class="si-stat"><span class="si-k">high</span><span class="si-v" id="siHigh">0</span></span>
        <span class="si-stat"><span class="si-k">wave</span><span class="si-v" id="siWave">1</span></span>
        <span class="si-stat"><span class="si-k">lives</span><span class="si-v" id="siLives">3</span></span>
      </div>

      <!-- Phase 2: active power-up indicator. Chips appear only while an
           ability is active; each shows its remaining duration. -->
      <div class="si-powers" id="siPowers" aria-live="polite"></div>

      <div class="si-stage">
        <canvas id="siCanvas" role="img"
          aria-label="Space Invaders play field. Your ship fires automatically; use left and right arrows or A and D to move, P to pause, R to restart. Touch controls appear below on mobile."></canvas>
        <div class="si-wave-banner" id="siWaveBanner" aria-hidden="true"></div>
        <!-- Phase 2: POWER-UP ACQUIRED notification, shown during the
             between-levels beat alongside LEVEL COMPLETE. -->
        <div class="si-wave-banner power" id="siPowerBanner" aria-hidden="true"></div>
        <div class="si-overlay hidden" id="siOverlay" aria-hidden="true">
          <div class="si-ov-inner">
            <div class="si-ov-title" id="siOvTitle">Space Invaders</div>
            <div class="si-ov-sub" id="siOvSub"></div>
            <button type="button" class="ft-btn primary" id="siOvBtn">Start Game</button>
            <div id="siOvLaunch"></div>
          </div>
        </div>
      </div>

      <!-- Persistent controls: usable at any time, not only in overlays. -->
      <div class="si-controls" role="group" aria-label="Game controls">
        <button type="button" class="ft-btn" id="siPauseBtn">Pause</button>
        <button type="button" class="ft-btn" id="siRestartBtn">Restart</button>
        <button type="button" class="ft-btn" id="siFsBtn" aria-pressed="false">Fullscreen</button>
        <button type="button" class="ft-btn si-sound" id="siSoundBtn" aria-pressed="false">
          <span class="si-snd-ico" aria-hidden="true">♪</span><span id="siSoundLbl">sound off</span>
        </button>
        <button type="button" class="ft-btn si-exit" id="siExit">Back to Games</button>
      </div>

      <!-- Phase 3/4: audio settings — independent music/effects volumes
           and mutes, persisted in localStorage. -->
      <div class="si-audio" id="siAudioPanel" aria-label="Audio settings">
        <div class="si-audio-row">
          <label class="si-audio-l" for="siMusicVol">music</label>
          <input type="range" id="siMusicVol" min="0" max="100" step="1" value="60"
                 aria-label="Music volume">
          <span class="si-audio-v" id="siMusicVolVal">60%</span>
          <button type="button" class="ft-btn si-mute" id="siMusicMute" aria-pressed="false">Music: on</button>
        </div>
        <div class="si-audio-row">
          <label class="si-audio-l" for="siSfxVol">effects</label>
          <input type="range" id="siSfxVol" min="0" max="100" step="1" value="70"
                 aria-label="Sound effects volume">
          <span class="si-audio-v" id="siSfxVolVal">70%</span>
          <button type="button" class="ft-btn si-mute" id="siSfxMute" aria-pressed="false">Effects: on</button>
        </div>
      </div>

      <div class="si-fs-notice" id="siFsNotice" role="status" aria-live="polite"></div>

      <div class="si-help">
        <span>←/→ or A/D move</span><span>firing is automatic</span><span>P pauses</span><span>R restarts</span><span>F fullscreen</span>
      </div>

      <!-- Touch controls: steering pads. Firing is automatic. -->
      <div class="si-touch" role="group" aria-label="Touch controls">
        <button type="button" class="si-pad" id="siLeft" aria-label="Move left">◀</button>
        <button type="button" class="si-pad si-auto" aria-disabled="true" tabindex="-1">AUTO-FIRE</button>
        <button type="button" class="si-pad" id="siRight" aria-label="Move right">▶</button>
      </div>
    </section>
  </div></div>`;

  /* ---- Bootstrap ----
   * Every listener is registered through siOn() so siTeardown() can remove
   * all of them when the sub-route changes. renderFunTime already tore down
   * any previous game, so this always starts from a clean slate. */
  siInit($('#siCanvas'));
  SIG.myEpoch = SIG.epoch;

  const siOn = (target, ev, fn, opts) => {
    target.addEventListener(ev, fn, opts);
    SIG.cleanup.push(() => target.removeEventListener(ev, fn, opts));
  };

  // Keyboard. Gameplay keys are captured only while the game area has
  // focus, so dashboard navigation and form fields stay unaffected.
  const GAME_KEYS = new Set(['ArrowLeft', 'ArrowRight', 'a', 'd', ' ', 'p', 'r', 'P', 'R', 'A', 'D', 'f', 'F', 'Escape']);
  siOn(window, 'keydown', e => {
    if (!GAME_KEYS.has(e.key)) return;
    // Never hijack keys while a text field is focused.
    const t = e.target;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    siAudioUnlock();                 // Phase 3/4: gesture unlocks audio
    if (e.key === 'p' || e.key === 'P') { siTogglePause(); e.preventDefault(); return; }
    if (e.key === 'r' || e.key === 'R') { siRestart(); e.preventDefault(); return; }
    if (e.key === 'f' || e.key === 'F') { siToggleFullscreen(); e.preventDefault(); return; }
    // Escape is owned by the shared session controller (ftSessionBind):
    // one press leaves fullscreen AND returns to Fun Time.

    if (e.key === ' ') {
      // Space would scroll the page; the game consumes it instead.
      if (SIG.g && SIG.g.state !== 'over') e.preventDefault();
    }
    SIG.keys[e.key] = true;
  });
  siOn(window, 'keyup', e => { delete SIG.keys[e.key]; });

  // Held-key state must not survive a blur or tab switch.
  siOn(window, 'blur', () => {
    SIG.keys = Object.create(null);
    SIG.touch = { left: false, right: false, fire: false };
    siPause();                       // safe pause on losing focus
  });
  siOn(document, 'visibilitychange', () => {
    if (document.hidden) { SIG.keys = Object.create(null); siPause(); }
  });

  // Persistent controls: usable at any time, not only in overlays.
  siOn($('#siPauseBtn'), 'click', () => siTogglePause());
  siOn($('#siRestartBtn'), 'click', () => siRestart());

  // Fullscreen: real Fullscreen API, entered only from this user gesture.
  siOn($('#siFsBtn'), 'click', () => { siToggleFullscreen(); });
  // Hide the control entirely when the browser cannot do real fullscreen;
  // the game stays fully playable in its dedicated inline view.
  if (!siFsSupported()) $('#siFsBtn').style.display = 'none';
  // Restore the dashboard layout whenever fullscreen ends — by Escape, the
  // browser control, or our own button. Registered via siOn so it is
  // removed with the view. ftFsReflect also clears every other game's fs
  // class defensively, so a direct SI→CR switch can never leave a stale one.
  siOn(document, 'fullscreenchange', () => { siFsReflect(); siResize(); });
  siOn(document, 'webkitfullscreenchange', () => { siFsReflect(); siResize(); });

  // Shared session lifecycle: automatic fullscreen from the card click,
  // Esc → back to Fun Time, dashboard chrome hidden while playing.
  ftSessionAfterMount('si', 'si-fs');
  ftSessionBind(siOn, () => siResize());

  // Sound toggle: one control for music + effects, muted by default.
  siOn($('#siSoundBtn'), 'click', () => { siAudioUnlock(); siSoundToggle(); });
  siSoundButton();                              // paint initial (off) state

  // Phase 3/4: audio settings — independent volumes and mutes, persisted.
  siOn($('#siMusicVol'), 'input', e => {
    siAudioUnlock();
    siAudioSetPrefs({ musicVol: Number(e.target.value) / 100 });
    siMusicEnsure();                            // start music if it was off
  });
  siOn($('#siSfxVol'), 'input', e => {
    siAudioUnlock();
    siAudioSetPrefs({ sfxVol: Number(e.target.value) / 100 });
    siSfx('power');                             // audible feedback of the new level
  });
  siOn($('#siMusicMute'), 'click', () => {
    siAudioUnlock();
    const on = !SIA.prefs.musicOn;
    siAudioSetPrefs({ musicOn: on });
    if (on) siMusicEnsure(); else siMusicStop();
  });
  siOn($('#siSfxMute'), 'click', () => {
    siAudioUnlock();
    siAudioSetPrefs({ sfxOn: !SIA.prefs.sfxOn });
    if (SIA.prefs.sfxOn) siSfx('power');
  });
  siAudioSettingsPaint();                       // reflect persisted prefs

  // Autoplay safety net: any interaction with the game view (key, pointer)
  // counts as the user gesture that unlocks the AudioContext. The listener
  // is capture-phase and passive so it never interferes with gameplay.
  siOn($('#siPanel'), 'pointerdown', () => siAudioUnlock(), { capture: true, passive: true });

  // Back to Games: navigate to the selector via the normal hash router, so
  // browser Back returns here. Teardown happens in renderFunTime.
  siOn($('#siExit'), 'click', () => { location.hash = '#funtime'; });

  // Overlay button: Start / Resume / Play Again, per the current overlay.
  siOn($('#siOvBtn'), 'click', () => {
    const kind = $('#siOvBtn').dataset.kind;
    if (kind === 'over') siRestart(); else siStart();
  });

  // Touch steering pads: pointer events cover touch + mouse uniformly,
  // with pointerdown/up/cancel so held movement works and stops correctly.
  // (No FIRE pad: firing is automatic.)
  const bindHold = (id, key) => {
    const el = $('#' + id);
    const on = e => { e.preventDefault(); SIG.touch[key] = true; };
    const off = e => { e.preventDefault(); SIG.touch[key] = false; };
    siOn(el, 'pointerdown', on);
    siOn(el, 'pointerup', off);
    siOn(el, 'pointercancel', off);
    siOn(el, 'pointerleave', off);
    // Prevent the browser treating a hold as a context menu / text select.
    siOn(el, 'contextmenu', e => e.preventDefault());
  };
  bindHold('siLeft', 'left');
  bindHold('siRight', 'right');

  // Resize: refit the canvas; logical state is untouched.
  siOn(window, 'resize', () => siResize());

  // Launch is automatic from the card click (FTSESS); no manual button.
  $('#siOvLaunch').hidden = true;

  // Start the single loop.
  siOverlay('ready');
  // Phase 3: menu music starts if audio was already unlocked by a prior
  // gesture (e.g. returning to the view); otherwise it starts on the first
  // interaction, via siAudioUnlock + siMusicEnsure in the frame loop.
  siMusicEnsure();
  SIG.lastT = performance.now();
  SIG.raf = requestAnimationFrame(siFrame);
}

/** Draw the 3×3 grid from FT state. Buttons are recreated each render, so
 *  no listener can ever double-bind. */
function ftRenderBoard() {
  const boardEl = $('#t3Board');
  if (!boardEl) return;
  boardEl.innerHTML = '';
  const winLine = FT.finished?.line || [];
  for (let i = 0; i < 9; i++) {
    const cell = document.createElement('button');
    cell.type = 'button';
    cell.className = 't3-cell' + (winLine.includes(i) ? ' win' : '');
    cell.dataset.i = String(i);
    // data-x / data-o drive the per-player colour rules in console.html
    // (X = theme green, O = blue) so the glyphs differ by more than shape.
    if (FT.board[i] === 'X') cell.dataset.x = '1';
    if (FT.board[i] === 'O') cell.dataset.o = '1';
    cell.textContent = FT.board[i];
    cell.disabled = FT.board[i] !== '' || !!FT.finished || FT.turn !== 'X' || FT.thinking;
    cell.setAttribute('aria-label',
      `Row ${Math.floor(i / 3) + 1}, column ${(i % 3) + 1}` +
      (FT.board[i] ? `, ${FT.board[i] === 'X' ? 'you' : 'the AI'}` : ', empty'));
    cell.addEventListener('click', () => {
      if (ftHumanMove(i)) { ftRenderBoard(); ftRenderStatus(); }
    });
    boardEl.appendChild(cell);
  }
}

function ftRenderScores() {
  const el = $('#t3Scores');
  if (!el) return;
  const s = FT.scores;
  el.innerHTML = `
    <div class="ft-score"><span class="ft-sk">you</span><span class="ft-sv">${s.you}</span></div>
    <div class="ft-score"><span class="ft-sk">ai</span><span class="ft-sv">${s.ai}</span></div>
    <div class="ft-score"><span class="ft-sk">draws</span><span class="ft-sv">${s.draw}</span></div>`;
}

function ftRenderStatus() {
  const el = $('#t3Status');
  if (!el) return;
  let text;
  if (FT.finished) {
    text = ftBanner(FT.finished);
  } else if (FT.thinking || FT.turn === 'O') {
    text = 'The AI is thinking…';
  } else {
    text = 'Your move — you are X.';
  }
  el.textContent = text;
  el.className = 'ft-status' + (FT.finished
    ? (FT.finished.kind === 'win' ? ' win' : FT.finished.kind === 'loss' ? ' loss' : ' draw')
    : '');
}


/* ============================================================
   HACK THE TERMINAL — cyberpunk terminal-hacking puzzle game
   Route: #funtime/ht · Namespace: HT / ht* functions / .ht- CSS.
   Everything is locally simulated; no real system is ever touched.
   Structure mirrors Space Invaders: one state object, an epoch guard,
   every listener registered through htOn(), teardown on route change.
   ============================================================ */

/* ---- 1. Audio (own context; never touches the SI engine) ------------- */
const HTA = {
  ctx: null, master: null, on: false,
};
function htAudioInit() {
  if (HTA.ctx) return true;
  const AC = window.AudioContext || window.webkitAudioContext;
  if (!AC) return false;
  try {
    HTA.ctx = new AC();
    HTA.master = HTA.ctx.createGain();
    HTA.master.gain.value = 0.4;
    HTA.master.connect(HTA.ctx.destination);
    return true;
  } catch { HTA.ctx = null; return false; }
}
function htBeep(freq, dur, vol, type) {
  if (!HTA.on || !HTA.ctx || !freq) return;
  const t = HTA.ctx.currentTime;
  const osc = HTA.ctx.createOscillator();
  const gain = HTA.ctx.createGain();
  osc.type = type || 'square';
  osc.frequency.value = freq;
  gain.gain.setValueAtTime(0.0001, t);
  gain.gain.exponentialRampToValueAtTime(vol, t + 0.01);
  gain.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(gain); gain.connect(HTA.master);
  osc.start(t); osc.stop(t + dur + 0.02);
}
function htSfx(kind) {
  switch (kind) {
    case 'key':  htBeep(1300, 0.025, 0.05, 'square'); break;
    case 'ok':   htBeep(660, 0.09, 0.16); setTimeout(() => htBeep(990, 0.12, 0.16), 90); break;
    case 'bad':  htBeep(180, 0.16, 0.18, 'sawtooth'); break;
    case 'win':  [523, 659, 784, 1047].forEach((f, i) =>
                   setTimeout(() => htBeep(f, 0.13, 0.15), i * 110)); break;
    case 'lose': [330, 262, 196].forEach((f, i) =>
                   setTimeout(() => htBeep(f, 0.18, 0.15, 'sawtooth'), i * 140)); break;
    case 'tick': htBeep(1500, 0.02, 0.04, 'sine'); break;
  }
}
function htAudioDispose() {
  if (HTA.ctx) { try { HTA.ctx.close(); } catch { /* already closed */ } }
  HTA.ctx = null; HTA.master = null;
}
function htSoundToggle() {
  HTA.on = !HTA.on;
  if (HTA.on) {
    if (!htAudioInit()) { HTA.on = false; }
    else if (HTA.ctx.state === 'suspended') HTA.ctx.resume().catch(() => {});
  }
  htSoundButton();
}
function htSoundButton() {
  const btn = $('#htSoundBtn'); if (!btn) return;
  btn.setAttribute('aria-pressed', String(HTA.on));
  btn.classList.toggle('on', HTA.on);
  const lbl = $('#htSoundLbl');
  const txt = HTA.on ? 'sound on' : 'sound off';
  if (lbl) lbl.textContent = txt;
  btn.title = HTA.on ? 'Sound on — click to mute' : 'Sound off — click to enable';
  btn.setAttribute('aria-label', btn.title);
}

/* ---- 2. Game state ---------------------------------------------------- */
const HT = {
  screen: 'intro',        // intro | brief | active | done | expired | victory
  mission: 0,             // 0..3 index into HT_MISSIONS
  score: 0,
  hintsUsed: 0,
  attempts: 0,            // wrong answers this mission
  correctFirstTry: 0,
  startedAt: 0,           // performance.now() when the mission timer began
  remaining: 0,           // seconds left (authoritative while running)
  timerId: 0,             // setInterval handle
  paused: false,
  pauseLeft: 0,           // remaining saved across a pause
  history: [],            // command history (Up/Down)
  histIdx: -1,
  puzzle: null,           // current mission's generated puzzle
  answered: false,        // guards duplicate score awards
  awarded: 0,             // points awarded for the current mission
  cleanup: [],
  epoch: 0,
  reducedMotion: false,
};

/* Mission definitions. `build()` returns a fresh puzzle each run so
 * answers cannot be memorised, and `check(puzzle, input)` validates
 * deterministically. `time` is the challenge budget in seconds. */
const HT_TIMES = [150, 180, 210, 240];
const HT_BASE = [300, 400, 500, 600];      // max points per mission

const HT_MISSIONS = [
  {
    id: 'access', title: 'Mission 1 · Terminal Access',
    time: () => HT_TIMES[0],
    build() {
      // Four distinct digits keep the puzzle solvable by reasoning from
      // the clues rather than brute force over 10000 codes.
      const digits = [];
      while (digits.length < 4) {
        const d = Math.floor(Math.random() * 10);
        if (!digits.includes(d)) digits.push(d);
      }
      const code = digits.join('');
      const sum = digits.reduce((a, b) => a + b, 0);
      const clues = [
        `The access code contains four digits.`,
        `The four digits are all different.`,
        `The sum of the digits is ${sum}.`,
        `The first digit is ${digits[0]} and the last digit is ${digits[3]}.`,
      ];
      return { code, clues, answer: code };
    },
    check(p, input) { return String(input).trim() === p.answer; },
    prompt: 'unlock <code>',
    hint(p) { return `Digit 2 is ${p.code[1]} and digit 3 is ${p.code[2]}.`; },
    solve(p) { return p.answer; },
  },
  {
    id: 'sequence', title: 'Mission 2 · Crack the Sequence',
    time: () => HT_TIMES[1],
    build() {
      // A 4-step arithmetic sequence with a random rule; the player sees
      // the first three terms and must produce the next two.
      const kind = Math.floor(Math.random() * 3);
      let terms, rule;
      if (kind === 0) {                       // add k
        const a = 2 + Math.floor(Math.random() * 8);
        const k = 3 + Math.floor(Math.random() * 9);
        terms = [a, a + k, a + 2 * k, a + 3 * k, a + 4 * k];
        rule = `add ${k} each step`;
      } else if (kind === 1) {                // multiply by k
        const a = 2 + Math.floor(Math.random() * 4);
        const k = 2 + Math.floor(Math.random() * 2);
        terms = [a, a * k, a * k * k, a * k ** 3, a * k ** 4];
        rule = `multiply by ${k} each step`;
      } else {                                // growing gap +k
        const a = 3 + Math.floor(Math.random() * 6);
        const k = 2 + Math.floor(Math.random() * 4);
        terms = [a, a + k, a + k + (k + 1), a + k + (k + 1) + (k + 2),
                 a + k + (k + 1) + (k + 2) + (k + 3)];
        rule = `the gaps between terms grow by 1 each step`;
      }
      return {
        shown: terms.slice(0, 3).join(', '),
        next: `${terms[3]}, ${terms[4]}`,
        answer: `${terms[3]},${terms[4]}`,
        rule,
      };
    },
    check(p, input) {
      // Accept "x, y", "x y" or "x,y," with any spacing.
      const nums = String(input).split(/[,\s]+/).filter(Boolean);
      if (nums.length !== 2) return false;
      const want = p.answer.split(',');
      return nums[0] === want[0] && nums[1] === want[1];
    },
    prompt: 'submit <a>, <b>',
    hint(p) { return `Rule: ${p.rule}. The 4th term is ${p.answer.split(',')[0]}.`; },
    solve(p) { return p.next; },
  },
  {
    id: 'decode', title: 'Mission 3 · Decode the Signal',
    time: () => HT_TIMES[2],
    build() {
      // Caesar cipher with a documented shift; the player is told the
      // shift, so decoding is a genuine (but fair) exercise.
      const words = ['FIREWALL', 'TERMINAL', 'NETWORK', 'CIPHER', 'PROTOCOL',
        'SYSTEM', 'SIGNAL', 'BACKDOOR', 'OVERRIDE', 'ENCRYPT'];
      const word = words[Math.floor(Math.random() * words.length)];
      const shift = 1 + Math.floor(Math.random() * 8);
      const enc = [...word].map(ch =>
        String.fromCharCode((ch.charCodeAt(0) - 65 + shift) % 26 + 65)).join('');
      return { word, shift, enc, answer: word };
    },
    check(p, input) {
      return String(input).trim().toUpperCase().replace(/[^A-Z]/g, '') === p.answer;
    },
    prompt: 'decode <word>',
    hint(p) { return `First letter decodes to "${p.word[0]}". The word has ${p.word.length} letters.`; },
    solve(p) { return p.answer; },
  },
  {
    id: 'firewall', title: 'Mission 4 · Firewall Bypass',
    time: () => HT_TIMES[3],
    build() {
      // Six numbered nodes; the clues uniquely determine an order of four.
      // Generated by shuffling and describing relationships that pin it.
      const pool = [1, 2, 3, 4, 5, 6, 7, 8];
      for (let i = pool.length - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [pool[i], pool[j]] = [pool[j], pool[i]];
      }
      const order = pool.slice(0, 4);
      const [a, b, c, d] = order;
      const clues = [
        `Node ${a} must be opened first.`,
        `Node ${c} is opened immediately after node ${b}.`,
        `Node ${d} is opened last, and its number is higher than node ${a}'s.`,
        `The four nodes to open are ${[...order].sort((x, y) => x - y).join(', ')}.`,
      ];
      return { order, clues, answer: order.join(',') };
    },
    check(p, input) {
      const nums = String(input).split(/[,\s>->]+/).filter(Boolean);
      if (nums.length !== 4) return false;
      return nums.map(Number).join(',') === p.answer;
    },
    prompt: 'bypass <n>,<n>,<n>,<n>',
    hint(p) { return `Second node is ${p.order[1]}, third is ${p.order[2]}.`; },
    solve(p) { return p.order.join(' -> '); },
  },
];

/* ---- 3. Terminal output helpers -------------------------------------- */
function htOut(html, cls) {
  const box = $('#htOut'); if (!box) return;
  const line = document.createElement('div');
  if (cls) line.className = cls;
  line.innerHTML = html;
  box.appendChild(line);
  // Keep the log bounded so a long session cannot grow the DOM forever.
  while (box.childElementCount > 400) box.removeChild(box.firstChild);
  box.scrollTop = box.scrollHeight;
}

function htGlitch() {
  if (HT.reducedMotion) return;
  const g = $('#htGlitch'); if (!g) return;
  g.classList.remove('on');
  void g.offsetWidth;                    // restart the animation
  g.classList.add('on');
}

/* ---- 4. Timer --------------------------------------------------------- */
function htTimerPaint() {
  const el = $('#htTimer'); if (!el) return;
  const v = el.querySelector('.si-v');
  if (HT.screen !== 'active' && HT.screen !== 'expired') {
    // Idle: no run or between missions — show a dash, not a fake zero.
    v.textContent = '–:––';
    el.classList.remove('warn', 'crit');
    return;
  }
  const m = Math.floor(HT.remaining / 60), s = Math.floor(HT.remaining % 60);
  v.textContent = `${m}:${String(s).padStart(2, '0')}`;
  el.classList.toggle('warn', HT.remaining <= 45 && HT.remaining > 15);
  el.classList.toggle('crit', HT.remaining <= 15);
}
function htTimerStart() {
  htTimerStop();
  HT.timerId = setInterval(() => {
    if (HT.paused) return;               // frozen while paused
    HT.remaining -= 1;
    if (HT.remaining <= 10 && HT.remaining > 0) htSfx('tick');
    htTimerPaint();
    if (HT.remaining <= 0) htExpire();
  }, 1000);
}
function htTimerStop() {
  if (HT.timerId) { clearInterval(HT.timerId); HT.timerId = 0; }
}
function htExpire() {
  htTimerStop();
  htGlitch(); htSfx('lose');
  HT.screen = 'expired';
  htOut('*** CONNECTION TIMED OUT ***', 'l-err l-big');
  htOut('The intrusion window closed before the lock was defeated.', 'l-dim');
  htOut(`Correct answer was: <b>${esc(HT_MISSIONS[HT.mission].solve(HT.puzzle))}</b>`, 'l-warn');
  htOut('Type <b>retry</b> to attempt this mission again, or <b>restart</b> to start over.', 'l-dim');
  htSetPrompt('retry');
  htHud();
}

/* ---- 5. HUD ----------------------------------------------------------- */
function htHud() {
  const set = (id, val) => { const e = $(id); if (e) e.textContent = val; };
  set('#htScore', String(HT.score));
  set('#htMissionNo', `${Math.min(HT.mission + 1, 4)}/4`);
  set('#htHints', String(HT.hintsUsed));
  set('#htTries', String(HT.attempts));
  htTimerPaint();
  htMissionHead();                    // keep the header band in sync too
}

/* ---- 6. Screen transitions -------------------------------------------- */
function htScreenIntro() {
  HT.screen = 'intro';
  htOut(`<span class="l-ok">HACK THE TERMINAL</span> <span class="l-dim">v1.0 — simulation</span>`, 'l-big');
  htOut('You are an independent security operator. A hostile corporation\'s', 'l-sys');
  htOut('mainframe has been seized. Four locks stand between you and root.', 'l-sys');
  htOut('&nbsp;');
  htOut('Solve all four missions to take control of the system.', 'l-cy');
  htOut('&nbsp;');
  htOut('Type <b>help</b> for commands · <b>start</b> to begin', 'l-dim');
  htSetPrompt('start');
  htHud();
}
function htScreenBrief() {
  const M = HT_MISSIONS[HT.mission];
  HT.screen = 'brief';
  htOut('&nbsp;');
  // One labelled section rule — the .ht-mhead band above the terminal
  // already carries the mission number and full title.
  htOut(`MISSION ${HT.mission + 1} · ${esc(M.title.replace(/^Mission \d+ · /, ''))}`, 'l-hd');
  htOut(`Time budget: <b>${M.time()}s</b> · Max points: <b>${HT_BASE[HT.mission]}</b>`, 'l-dim');
  htOut('Attempts are unlimited, but wrong answers cost accuracy.', 'l-dim');
  htOut('&nbsp;');
  htOut('Type <b>begin</b> to open the challenge.', 'l-cy');
  htSetPrompt('begin');
  htMissionHead();
}
function htScreenActive() {
  const M = HT_MISSIONS[HT.mission];
  HT.screen = 'active';
  HT.puzzle = M.build();
  HT.answered = false; HT.awarded = 0; HT.attempts = 0;
  HT.remaining = M.time();
  htOut('&nbsp;');
  htOut(`— ${esc(M.title)} —`, 'l-vio');
  // Mission-specific narrative + the puzzle itself. l-sim marks lines the
  // *simulation* prints, so they read differently from the operator's own
  // echoed commands (.l-cmd).
  if (M.id === 'access') {
    htOut('system.connect()', 'l-sim');
    htOut('Connection established.', 'l-ok');
    htOut('access.verify', 'l-sim');
    htOut('<b>ACCESS DENIED</b> — 4-digit code required', 'l-err');
  } else if (M.id === 'sequence') {
    htOut('seq.intercept --channel 7', 'l-sim');
    htOut('Intercepted fragment of the handshake sequence:', 'l-cy');
    htOut(`&nbsp;&nbsp;<b>${esc(HT.puzzle.shown)}</b>, ?, ?`, 'l-big');
  } else if (M.id === 'decode') {
    htOut('signal.sniff --freq 141.7', 'l-sim');
    htOut('Encrypted burst received. Cipher: <b>Caesar</b>, shift '
        + `<b>+${HT.puzzle.shift}</b> (each letter shifted forward).`, 'l-cy');
    htOut(`&nbsp;&nbsp;<b>${esc(HT.puzzle.enc)}</b>`, 'l-big');
    htOut('Decode it back to the original word.', 'l-dim');
  } else {
    htOut('firewall.map', 'l-sim');
    htOut('Six security nodes detected. Four guard the path.', 'l-cy');
  }
  htOut('&nbsp;');
  htOut('INTEL', 'l-hd');
  for (const c of HT.puzzle.clues || []) htOut(`<span class="ht-clue">${esc(c)}</span>`, '');
  if (M.id === 'sequence') {
    htOut(`<span class="ht-clue">Give the NEXT TWO terms, separated by a comma.</span>`, '');
  } else if (M.id === 'decode') {
    htOut(`<span class="ht-clue">Enter the decoded English word.</span>`, '');
  } else if (M.id === 'firewall') {
    htOut(`<span class="ht-clue">Enter the four node numbers in opening order, comma-separated.</span>`, '');
  } else {
    htOut(`<span class="ht-clue">Enter the 4-digit access code.</span>`, '');
  }
  htOut('&nbsp;');
  htOut(`Answer with: <b>${esc(M.prompt)}</b>`, 'l-dim');
  htOut('Stuck? <b>hint</b> costs half the mission points.', 'l-dim');
  htSetPrompt(M.prompt.replace(/[<>]/g, ''));
  htTimerStart();
  htTimerPaint();
  htHud();
  htMissionHead();
}
function htScreenDone(correct) {
  const M = HT_MISSIONS[HT.mission];
  htTimerStop();
  HT.screen = 'done';
  if (correct) {
    htOut('&nbsp;');
    htOut('*** LOCK DEFEATED ***', 'l-ok l-big');
    htOut(`+${HT.awarded} points`, 'l-cy');
    if (HT.mission >= HT_MISSIONS.length - 1) {
      htScreenVictory();
      return;
    }
    htOut('&nbsp;');
    htOut(`Mission ${HT.mission + 1} complete. Type <b>next</b> for the following lock.`, 'l-dim');
    htSetPrompt('next');
  }
  htHud();
}
function htScreenVictory() {
  HT.screen = 'victory';
  htOut('&nbsp;');
  htOut('███████╗ ██████╗ ██╗  ██╗ ██████╗ ███╗   ██╗', 'l-ok');
  htOut('██╔════╝██╔═══██╗██║ ██╔╝██╔═══██╗████╗  ██║', 'l-ok');
  htOut('█████╗  ██║   ██║█████╔╝ ██║   ██║██╔██╗ ██║', 'l-ok');
  htOut('██╔══╝  ██║   ██║██╔═██╗ ██║   ██║██║╚██╗██║', 'l-ok');
  htOut('██║     ╚██████╔╝██║  ██╗╚██████╔╝██║ ╚████║', 'l-ok');
  htOut('╚═╝      ╚═════╝ ╚═╝  ╚═╝ ╚═════╝ ╚═╝  ╚═══╝', 'l-ok');
  htOut('&nbsp;');
  htOut('All four locks defeated. Root access granted.', 'l-cy');
  htOut('&nbsp;');
  const acc = HT.attemptsTotal ? Math.round(100 * HT.correctFirstTry / HT.attemptsTotal) : 100;
  htOut('MISSION SUMMARY', 'l-hd');
  // Max achievable = base (1800) + the 40% time bonus on every mission.
  const maxScore = HT_BASE.reduce((a, b) => a + b, 0) * 1.4;
  htOut(`Final score ....... <b>${HT.score}</b> / ${Math.round(maxScore)}`, 'l-ok');
  htOut(`First-try solves .. <b>${HT.correctFirstTry}/4</b>`, 'l-sys');
  htOut(`Accuracy .......... <b>${acc}%</b>`, 'l-sys');
  htOut(`Hints used ........ <b>${HT.hintsUsed}</b>`, 'l-sys');
  htOut('&nbsp;');
  htOut('Type <b>restart</b> to play again.', 'l-dim');
  htSetPrompt('restart');
  htSfx('win');
  htHud();
  htMissionHead();
}

/** Paint the persistent mission header band above the terminal: mission
 *  number, title and the live phase. Pure presentation — it reads only
 *  existing HT state and never mutates it. */
function htMissionHead() {
  const head = $('#htMhead'); if (!head) return;
  const started = HT.screen !== 'intro';
  const idx = Math.min(HT.mission, HT_MISSIONS.length - 1);
  const M = HT_MISSIONS[idx];
  const numEl = head.querySelector('.ht-mnum');
  const titleEl = head.querySelector('.ht-mtitle');
  const metaEl = head.querySelector('.ht-mmeta');
  if (numEl) numEl.textContent = started ? `LOCK ${idx + 1}/${HT_MISSIONS.length}` : 'STANDBY';
  if (titleEl) titleEl.textContent = started ? M.title : 'Awaiting operator';
  if (metaEl) {
    const phase = { intro: 'standby', brief: 'briefing', active: HT.paused ? 'paused' : 'live',
                    done: 'cleared', expired: 'timeout', victory: 'root' }[HT.screen] || HT.screen;
    metaEl.textContent = `${phase} · ${M.time()}s budget`;
  }
}

/* ---- 7. Prompt -------------------------------------------------------- */
function htSetPrompt(text) {
  const ps = $('#htPs'); const inp = $('#htInput');
  if (!ps || !inp) return;
  ps.textContent = '$';
  inp.placeholder = text;
}

/* ---- 8. Command processing -------------------------------------------- */
function htHelp() {
  htOut('AVAILABLE COMMANDS', 'l-hd');
  htOut('  help        this list', 'l-sys');
  htOut('  status      mission, score and timer state', 'l-sys');
  htOut('  scan        probe the current lock for intel', 'l-sys');
  htOut('  hint        reveal a clue (costs half the mission points)', 'l-sys');
  htOut('  clear       wipe the terminal output', 'l-sys');
  htOut('  pause / resume   freeze or continue the countdown', 'l-sys');
  htOut('  retry       replay the current mission (score kept)', 'l-sys');
  htOut('  restart     abandon the run and start over', 'l-sys');
  htOut('&nbsp;');
  const M = HT_MISSIONS[HT.mission];
  if (HT.screen === 'active') htOut(`Answer format: <b>${esc(M.prompt)}</b>`, 'l-cy');
  else if (HT.screen === 'intro') htOut('Type <b>start</b> to begin.', 'l-cy');
  else if (HT.screen === 'brief') htOut('Type <b>begin</b> to open the challenge.', 'l-cy');
  else if (HT.screen === 'done') htOut('Type <b>next</b> for the next mission.', 'l-cy');
  else if (HT.screen === 'expired') htOut('Type <b>retry</b> to try this mission again.', 'l-cy');
  else if (HT.screen === 'victory') htOut('Type <b>restart</b> to play again.', 'l-cy');
}
function htStatus() {
  htOut('STATUS', 'l-hd');
  htOut(`Mission .... ${HT.screen === 'intro' ? 'not started'
    : `${Math.min(HT.mission + 1, 4)} of 4 (${HT_MISSIONS[HT.mission].title})`}`, 'l-sys');
  htOut(`Phase ...... ${HT.screen}${HT.paused ? ' (paused)' : ''}`, 'l-sys');
  htOut(`Score ...... <b>${HT.score}</b>`, 'l-ok');
  htOut(`Timer ...... ${HT.screen === 'active' ? `${Math.max(0, Math.ceil(HT.remaining))}s remaining` : 'idle'}`, 'l-sys');
  htOut(`Hints ...... ${HT.hintsUsed} used · Attempts this mission: ${HT.attempts}`, 'l-sys');
}
function htScan() {
  if (HT.screen === 'intro') { htOut('No target connected yet. Type <b>start</b>.', 'l-warn'); return; }
  const M = HT_MISSIONS[HT.mission];
  htOut(`Scanning ${M.id.toUpperCase()} node…`, 'l-dim');
  const flavor = {
    access: ['Port 22 open (ssh)', 'Port 8080 open (http-proxy)', 'Keypad daemon listening'],
    sequence: ['Handshake uses a numeric ladder', 'Fragment shows 3 of 5 terms', 'Ladder rule is deterministic'],
    decode: ['Burst encoded with a classical cipher', 'Key space is tiny (shift cipher)', 'Payload is a single English word'],
    firewall: ['6 nodes, 4 on the critical path', 'Nodes must be opened in strict order', 'Opening order follows the intel clues'],
  }[M.id];
  for (const f of flavor) htOut(`  • ${f}`, 'l-cy');
  if (HT.puzzle && HT.puzzle.clues) {
    htOut('Intel:', 'l-dim');
    for (const c of HT.puzzle.clues) htOut(`  » ${esc(c)}`, 'l-cy');
  }
}
function htHint() {
  if (HT.screen !== 'active') {
    htOut('Hints are only available during an active challenge.', 'l-warn');
    return;
  }
  const M = HT_MISSIONS[HT.mission];
  HT.hintsUsed++;
  // A hint halves the points still available for this mission.
  HT.hintPenalty = (HT.hintPenalty || 0) + 0.5;
  htOut(`HINT: ${esc(M.hint(HT.puzzle))}`, 'l-vio l-big');
  htOut('(mission reward halved)', 'l-dim');
  htSfx('key');
  htHud();
}
function htClear() {
  const box = $('#htOut'); if (!box) return;
  box.innerHTML = '';
  htOut('Terminal cleared.', 'l-dim');
}

/** Submit an answer for the active mission. */
function htSubmit(raw) {
  if (HT.screen !== 'active') {
    htOut('No active challenge. Start or resume a mission first.', 'l-warn');
    return;
  }
  const M = HT_MISSIONS[HT.mission];
  // The dispatcher passes the whole line. Strip the answer verb only when
  // the line actually starts with one — a bare answer ("8327") must pass
  // through untouched.
  const VERBS = ['unlock', 'submit', 'decode', 'bypass'];
  const first = raw.trim().split(/\s+/)[0].toLowerCase();
  const value = (VERBS.includes(first) && raw.trim().includes(' ')
    ? raw.trim().slice(raw.trim().indexOf(' ') + 1)
    : raw).trim();
  if (!value) {
    htOut(`Usage: <b>${esc(M.prompt)}</b>`, 'l-warn');
    return;
  }
  if (HT.answered) {                                  // duplicate guard
    htOut('This lock is already defeated. Type <b>next</b>.', 'l-dim');
    return;
  }
  const ok = M.check(HT.puzzle, value);
  if (!ok) {
    HT.attempts++;
    htGlitch(); htSfx('bad');
    htOut(`REJECTED — ${esc(value)}`, 'l-err');
    htOut(`${M.id === 'access' ? 'The keypad buzzes. That code is not right.'
      : 'The system rejects that input.'} Attempt ${HT.attempts}.`, 'l-dim');
    htHud();
    return;
  }
  // ---- correct: award points exactly once ---------------------------
  HT.answered = true;
  const timeBonus = Math.round((HT.remaining / M.time()) * HT_BASE[HT.mission] * 0.4);
  const penalty = Math.min(0.9, (HT.hintPenalty || 0) + HT.attempts * 0.1);
  HT.awarded = Math.max(50, Math.round(HT_BASE[HT.mission] * (1 - penalty)) + timeBonus);
  if (HT.attempts === 0 && !(HT.hintPenalty > 0)) HT.correctFirstTry++;
  HT.score += HT.awarded;
  HT.attemptsTotal = (HT.attemptsTotal || 0) + 1;
  htSfx('ok');
  htScreenDone(true);
}

/** Central dispatcher. Returns nothing; prints its own feedback. */
function htCommand(raw) {
  const line = raw.trim();
  if (!line) return;
  htOut(`$ ${esc(line)}`, 'l-cmd');
  const [cmd, ...rest] = line.split(/\s+/);
  const arg = rest.join(' ');
  const M = HT_MISSIONS[HT.mission];

  switch (cmd.toLowerCase()) {
    case 'help': case '?': htHelp(); break;
    case 'status': htStatus(); break;
    case 'scan': htScan(); break;
    case 'hint': htHint(); break;
    case 'clear': case 'cls': htClear(); break;
    case 'pause':
      if (HT.screen !== 'active') htOut('Nothing to pause.', 'l-warn');
      else if (HT.paused) htOut('Already paused.', 'l-dim');
      else { HT.paused = true; htOut('** PAUSED — countdown frozen **', 'l-warn'); htMissionHead(); }
      break;
    case 'resume':
      if (!HT.paused) htOut('Not paused.', 'l-dim');
      else { HT.paused = false; htOut('Resumed.', 'l-ok'); htMissionHead(); }
      break;
    case 'start':
      if (HT.screen !== 'intro') htOut('A run is already in progress. Use <b>restart</b> to start over.', 'l-warn');
      else { HT.mission = 0; HT.score = 0; HT.hintsUsed = 0; HT.correctFirstTry = 0;
             HT.attemptsTotal = 0; HT.hintPenalty = 0; htScreenBrief(); }
      break;
    case 'begin':
      if (HT.screen !== 'brief') htOut('No briefing pending. <b>start</b> begins a run.', 'l-warn');
      else htScreenActive();
      break;
    case 'next':
      if (HT.screen !== 'done') htOut('No completed mission pending.', 'l-warn');
      else { HT.mission++; HT.hintPenalty = 0; htScreenBrief(); }
      break;
    case 'retry':
      if (HT.screen !== 'expired') htOut('Nothing to retry.', 'l-warn');
      else htScreenActive();
      break;
    case 'restart':
      htRestartRun();
      break;
    // Answer verbs (mission-specific).
    case 'unlock': case 'submit': case 'decode': case 'bypass':
      htSubmit(line);
      break;
    default:
      // Bare answer in the active screen is accepted too.
      if (HT.screen === 'active') { htSubmit(line); break; }
      htGlitch();
      htOut(`Unknown command: <b>${esc(cmd)}</b>`, 'l-err');
      htOut('This shell is a simulation — only game commands run here.');
      htOut('Type <b>help</b> for the command list.', 'l-dim');
  }
}
function htRestartRun() {
  htTimerStop();
  HT.mission = 0; HT.score = 0; HT.hintsUsed = 0; HT.attempts = 0;
  HT.correctFirstTry = 0; HT.attemptsTotal = 0; HT.hintPenalty = 0;
  HT.paused = false; HT.answered = false; HT.history = []; HT.histIdx = -1;
  const box = $('#htOut'); if (box) box.innerHTML = '';
  htScreenIntro();
}

/* ---- 9. View + lifecycle ---------------------------------------------- */
function renderHtView() {
  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <div class="ft-game-top">
      <a class="ft-back" href="#funtime" title="Back to the games">← back to games</a>
      <div class="ft-game-title">Hack the Terminal</div>
    </div>

    <section class="ft-panel ft-game ht-wide" id="htPanel" aria-label="Hack the Terminal game">
      <header class="ft-panel-h">
        <div class="ws-ico ft-ico">${ICONS.funtime}</div>
        <div class="ft-panel-t">
          <div class="ft-name">Hack the Terminal</div>
          <div class="ft-blurb">Four locks. One root shell. Solve the puzzles before the connection times out.</div>
        </div>
      </header>

      <div class="ht-mhead" id="htMhead" aria-live="off">
        <span class="ht-mnum">STANDBY</span>
        <span class="ht-mtitle">Awaiting operator</span>
        <span class="ht-mmeta">standby · 150s budget</span>
      </div>

      <div class="ht-hud" role="status" aria-live="off">
        <span class="si-stat"><span class="si-k">score</span><span class="si-v" id="htScore">0</span></span>
        <span class="si-stat"><span class="si-k">mission</span><span class="si-v" id="htMissionNo">–</span></span>
        <span class="si-stat ht-timer" id="htTimer"><span class="si-k">time</span><span class="si-v">–:––</span></span>
        <span class="si-stat"><span class="si-k">hints</span><span class="si-v" id="htHints">0</span></span>
        <span class="si-stat"><span class="si-k">tries</span><span class="si-v" id="htTries">0</span></span>
      </div>

      <div class="ht-term">
        <div class="ht-term-bar">
          <span class="ht-dots" aria-hidden="true"><i></i><i></i><i></i></span>
          <span class="ht-term-title">operator@loop — intrusion shell</span>
          <span class="ht-net"><i aria-hidden="true"></i><span id="htNetState">SIMULATED</span></span>
        </div>
        <div class="ht-out" id="htOut" role="log" aria-label="Terminal output"
             aria-live="polite" tabindex="0"></div>
        <div class="ht-scan" aria-hidden="true"></div>
        <div class="ht-glitch" id="htGlitch" aria-hidden="true"></div>
        <div class="ht-in-row">
          <span class="ht-ps" id="htPs" aria-hidden="true">$</span>
          <input class="ht-input" id="htInput" type="text" autocomplete="off"
                 autocapitalize="off" spellcheck="false"
                 placeholder="start"
                 aria-label="Terminal command input" />
          <span class="ht-cur" aria-hidden="true"></span>
        </div>

        <div class="ht-controls" role="group" aria-label="Game controls">
          <button type="button" class="ft-btn primary" id="htStartBtn">Start Mission</button>
          <button type="button" class="ft-btn" id="htPauseBtn">Pause</button>
          <button type="button" class="ft-btn" id="htRestartBtn">Restart</button>
          <button type="button" class="ft-btn si-sound" id="htSoundBtn" aria-pressed="false">
            <span class="si-snd-ico" aria-hidden="true">♪</span><span id="htSoundLbl">sound off</span>
          </button>
          <button type="button" class="ft-btn" id="htFsBtn" aria-pressed="false">Fullscreen</button>
          <button type="button" class="ft-btn ht-exit" id="htExit">Back to Games</button>
        </div>
      </div>

      <div class="si-help">
        <span>type commands + Enter</span><span>↑/↓ history</span>
        <span>help · status · scan · hint · clear</span>
      </div>
    </section>
  </div></div>`;


  /* Bootstrap. Every listener goes through htOn() so htTeardown() can
   * remove them all when the sub-route changes. */
  HT.epoch++;
  const myEpoch = HT.epoch;
  HT.reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  HT.cleanup = [];

  const htOn = (target, ev, fn, opts) => {
    target.addEventListener(ev, fn, opts);
    HT.cleanup.push(() => target.removeEventListener(ev, fn, opts));
  };

  // Fresh run each visit — the game must never start by itself.
  HT.screen = 'intro'; HT.mission = 0; HT.score = 0; HT.hintsUsed = 0;
  HT.attempts = 0; HT.correctFirstTry = 0; HT.attemptsTotal = 0;
  HT.hintPenalty = 0; HT.paused = false; HT.answered = false;
  HT.history = []; HT.histIdx = -1; HT.timerId = 0;
  // Sound is muted by default on every entry, so the button always starts
  // in a state that matches reality (a previous visit's preference must
  // not leave the toggle out of sync with the disposed audio context).
  HTA.on = false;

  const input = $('#htInput');
  htScreenIntro();
  htSoundButton();

  // Execute a command from the input, record history.
  const run = () => {
    const v = input.value;
    if (!v.trim()) return;
    HT.history.push(v);
    HT.histIdx = HT.history.length;
    htCommand(v);
    input.value = '';
    input.focus();
  };
  htOn(input, 'keydown', e => {
    if (e.key === 'Enter') { e.preventDefault(); run(); }
    else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (HT.history.length) {
        HT.histIdx = Math.max(0, HT.histIdx - 1);
        input.value = HT.history[HT.histIdx] || '';
      }
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (HT.history.length) {
        HT.histIdx = Math.min(HT.history.length, HT.histIdx + 1);
        input.value = HT.histIdx >= HT.history.length ? '' : HT.history[HT.histIdx];
      }
    }
  });
  // Keystroke blips (very quiet, and only when sound is enabled).
  htOn(input, 'input', () => { if (myEpoch === HT.epoch) htSfx('key'); });

  // Buttons drive the same commands, so both interaction paths share one
  // implementation and one validation path.
  htOn($('#htStartBtn'), 'click', () => {
    if (HT.screen === 'intro') htCommand('start');
    else if (HT.screen === 'brief') htCommand('begin');
    else if (HT.screen === 'done') htCommand('next');
    else if (HT.screen === 'expired') htCommand('retry');
    else if (HT.screen === 'victory') htCommand('restart');
    input.focus();
  });
  htOn($('#htPauseBtn'), 'click', () => {
    htCommand(HT.paused ? 'resume' : 'pause');
    const b = $('#htPauseBtn'); if (b) b.textContent = HT.paused ? 'Resume' : 'Pause';
    input.focus();
  });
  htOn($('#htRestartBtn'), 'click', () => { htCommand('restart'); input.focus(); });
  htOn($('#htSoundBtn'), 'click', () => { htSoundToggle(); input.focus(); });
  htOn($('#htExit'), 'click', () => { location.hash = '#funtime'; });

  // Fullscreen: the shared FTFS convention (real Fullscreen API, entered
  // only from this user gesture). The terminal keeps keyboard focus so
  // typing continues to work in fullscreen.
  htFsBind(htOn, input);

  // Launch is automatic from the card click (FTSESS); no manual button.

  // Shared session lifecycle: automatic fullscreen from the card click,
  // Esc → back to Fun Time, dashboard chrome hidden while playing.
  ftSessionAfterMount('ht', 'ht-fs');
  ftSessionBind(htOn, null);

  // Escape is owned by ftSessionBind above (one press → Fun Time). Never
  // steals Escape while the player is typing in the terminal — the
  // browser's own text-editing behaviour for Escape stays untouched.

  // Focus the terminal when the panel is clicked (but never steal focus
  // from the buttons themselves).
  htOn($('#htPanel'), 'click', e => {
    if (e.target.closest('button, a, input')) return;
    input.focus();
  });

  // Pause automatically when the tab is hidden, so the countdown cannot
  // run away while the player is elsewhere.
  htOn(document, 'visibilitychange', () => {
    if (document.hidden && HT.screen === 'active' && !HT.paused) {
      htCommand('pause');
      const b = $('#htPauseBtn'); if (b) b.textContent = 'Resume';
    }
  });
}

/** Fullscreen helpers for Hack the Terminal (shared FTFS system). The
 *  terminal input keeps focus so typing works in fullscreen. */
function htFsBind(htOn, input) {
  const reflect = () => {
    ftFsReflect('ht', 'ht-fs', '#htFsBtn');
    // Keep the caret available after entering/leaving fullscreen.
    if (input && HT.screen === 'active' && !HT.paused) {
      try { input.focus({ preventScroll: true }); } catch { /* detached */ }
    }
  };
  htOn($('#htFsBtn'), 'click', async () => {
    await ftToggleFullscreen('ht', 'ht-fs', null);
    // Focus back into the terminal after the transition settles.
    setTimeout(() => {
      if (input && HT.epoch === HT.myEpoch && HT.screen === 'active' && !HT.paused) {
        try { input.focus({ preventScroll: true }); } catch { /* detached */ }
      }
    }, 120);
  });
  htOn(document, 'fullscreenchange', reflect);
  htOn(document, 'webkitfullscreenchange', reflect);
  if (!ftFsSupported()) {
    const b = $('#htFsBtn');
    if (b) b.style.display = 'none';
  }
}

/** Tear down the whole game: timers, listeners, audio. Idempotent. */
function htTeardown() {
  HT.epoch++;
  htTimerStop();
  for (const off of HT.cleanup) { try { off(); } catch { /* gone */ } }
  HT.cleanup = [];
  HT.paused = false;
  htAudioDispose();
  // Leaving the route must also leave fullscreen and restore the layout.
  document.body.classList.remove('ht-fs');
  ftExitFullscreenQuiet();
  ftSessionRestore();               // restore dashboard chrome
}


/* ============================================================
   SOKET STREET RACING: CYBER RUSH — pseudo-3D canvas racer
   Route: #funtime/cr · Namespace: CR / cr* functions / .cr- CSS.
   Rendering: classic segment-based pseudo-3D on Canvas 2D (the same
   approach as 16-bit racers) — no dependencies, cheap to draw, and it
   scales to any resolution because everything is projected from logical
   world units.

   World model
   -----------
   The track is a list of segments, each SEG_LEN world units long. Each
   segment stores its curve (steering pull) and y-hill. The camera sits
   behind the player at `camDepth`; every frame we walk forward from the
   camera position, projecting each segment's near/far edges to screen x/y
   with perspective divide. Sprites (traffic, roadside pylons) are drawn
   back-to-front with the same projection, so scale and clipping fall out
   of the math for free.

   Lifecycle mirrors Space Invaders: one state object, an epoch guard that
   invalidates stale loops, every listener registered through crOn(), and
   crTeardown() on any route change.
   ============================================================ */

/* ---- 1. Audio (own context; separate from SI and HT engines) ---------- */
const CRA = {
  ctx: null, master: null, on: false, tyre: null,
};
function crAudioInit() {
  if (CRA.ctx) return true;
  const AC = window.AudioContext || window.webkitAudioContext;
  if (!AC) return false;
  try {
    CRA.ctx = new AC();
    CRA.master = CRA.ctx.createGain();
    CRA.master.gain.value = 0.35;
    CRA.master.connect(CRA.ctx.destination);
    return true;
  } catch { CRA.ctx = null; return false; }
}
function crBeep(freq, dur, vol, type) {
  if (!CRA.on || !CRA.ctx || !freq) return;
  const t = CRA.ctx.currentTime;
  const osc = CRA.ctx.createOscillator();
  const gain = CRA.ctx.createGain();
  osc.type = type || 'square';
  osc.frequency.value = freq;
  gain.gain.setValueAtTime(0.0001, t);
  gain.gain.exponentialRampToValueAtTime(vol, t + 0.01);
  gain.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(gain); gain.connect(CRA.master);
  osc.start(t); osc.stop(t + dur + 0.02);
}
/* PHASE 6 · gear-aware engine note. rpm (0..1) is the within-gear rev
 * position computed by the transmission model in crStep; the drone pitch
 * now saws up through each gear and DROPS on upshift — the classic
 * race-engine sound — instead of one long monotone sweep. */
function crEngineUpdate(speedRatio, rpm, gear) {
  if (!CRA.on || !CRA.ctx || !CRA.engine) return;
  const t = CRA.ctx.currentTime;
  const r = (rpm === undefined) ? speedRatio : Math.max(0, Math.min(1, rpm));
  // Base rises slightly with gear; rev position drives the sweep.
  const base = 52 + (gear || 1) * 6;
  const f = base + r * 150 + speedRatio * 30;
  CRA.engine.frequency.setTargetAtTime(f, t, 0.06);
  CRA.engineGain.gain.setTargetAtTime(0.05 + speedRatio * 0.06, t, 0.1);
}
/** Gear-shift blip: short throttle-lift dip then a snap back up. */
function crSfxGear() {
  if (!CRA.on || !CRA.ctx) return;
  crBeep(180, 0.05, 0.10, 'square');
  setTimeout(() => crBeep(320, 0.07, 0.09, 'square'), 55);
}
/** Drift tyre squeal: a looping filtered noise band while sliding.
 *  Started/stopped by the drift state; created lazily, never overlapped. */
function crDriftAudio(on) {
  if (!CRA.on || !CRA.ctx) { if (CRA.tyre) crDriftAudioStop(); return; }
  if (on && !CRA.tyre) {
    try {
      // Noise buffer: 0.4s of white noise, looped.
      const len = Math.floor(CRA.ctx.sampleRate * 0.4);
      const buf = CRA.ctx.createBuffer(1, len, CRA.ctx.sampleRate);
      const d = buf.getChannelData(0);
      for (let i = 0; i < len; i++) d[i] = Math.random() * 2 - 1;
      const src = CRA.ctx.createBufferSource();
      src.buffer = buf; src.loop = true;
      const bp = CRA.ctx.createBiquadFilter();
      bp.type = 'bandpass'; bp.frequency.value = 900; bp.Q.value = 1.4;
      const gain = CRA.ctx.createGain(); gain.gain.value = 0.0;
      src.connect(bp); bp.connect(gain); gain.connect(CRA.master);
      src.start();
      gain.gain.setTargetAtTime(0.055, CRA.ctx.currentTime, 0.08);
      CRA.tyre = { src, gain };
    } catch { CRA.tyre = null; }
  } else if (!on && CRA.tyre) {
    crDriftAudioStop();
  }
}
function crDriftAudioStop() {
  if (!CRA.tyre) return;
  try {
    CRA.tyre.gain.gain.setTargetAtTime(0, CRA.ctx.currentTime, 0.05);
    const s = CRA.tyre.src;
    setTimeout(() => { try { s.stop(); } catch { /* done */ } }, 160);
  } catch { /* gone */ }
  CRA.tyre = null;
}
function crEngineStart() {
  if (!CRA.on || !CRA.ctx || CRA.engine) return;
  const osc = CRA.ctx.createOscillator();
  const gain = CRA.ctx.createGain();
  osc.type = 'sawtooth';
  osc.frequency.value = 55;
  gain.gain.value = 0.0;
  osc.connect(gain); gain.connect(CRA.master);
  osc.start();
  CRA.engine = osc; CRA.engineGain = gain;
}
function crEngineStop() {
  if (CRA.engine) { try { CRA.engine.stop(); } catch { /* already stopped */ } }
  CRA.engine = null; CRA.engineGain = null;
}
function crSfx(kind) {
  switch (kind) {
    case 'ui':    crBeep(880, 0.05, 0.10); break;
    case 'nitro': crBeep(300, 0.28, 0.14, 'sawtooth');
                  setTimeout(() => crBeep(600, 0.22, 0.12, 'sawtooth'), 60); break;
    case 'crash': [220, 150, 90].forEach((f, i) =>
                    setTimeout(() => crBeep(f, 0.16, 0.16, 'sawtooth'), i * 70)); break;
    case 'pass':  crBeep(1240, 0.045, 0.07); break;
    case 'rumble': crBeep(70, 0.05, 0.05, 'sawtooth'); break;
    case 'win':   [523, 659, 784, 1047].forEach((f, i) =>
                    setTimeout(() => crBeep(f, 0.13, 0.14), i * 110)); break;
    case 'lose':  [392, 311, 233].forEach((f, i) =>
                    setTimeout(() => crBeep(f, 0.18, 0.14, 'sawtooth'), i * 140)); break;
    case 'count': crBeep(700, 0.09, 0.13); break;
    case 'go':    crBeep(1180, 0.20, 0.15); break;
    // Drift payout: rising two-tone chirp — reads as "banked points".
    case 'drift': [660, 990].forEach((f, i) =>
                    setTimeout(() => crBeep(f, 0.10, 0.11, 'triangle'), i * 90)); break;
    // Near miss: sharp whoosh past.
    case 'nearmiss': crBeep(1600, 0.05, 0.10, 'triangle');
                     setTimeout(() => crBeep(900, 0.07, 0.08, 'triangle'), 45); break;
    // PHASE 6 · unlock fanfare: rising arpeggio with a sparkle on top.
    case 'unlock': [523, 659, 784, 1047, 1319].forEach((f, i) =>
                     setTimeout(() => crBeep(f, 0.12, 0.13, 'triangle'), i * 85)); break;
  }
}
function crAudioDispose() {
  crDriftAudioStop();
  crEngineStop();
  if (CRA.ctx) { try { CRA.ctx.close(); } catch { /* already closed */ } }
  CRA.ctx = null; CRA.master = null;
}
function crSoundToggle() {
  CRA.on = !CRA.on;
  if (CRA.on) {
    if (!crAudioInit()) { CRA.on = false; }
    else {
      if (CRA.ctx.state === 'suspended') CRA.ctx.resume().catch(() => {});
      // The engine drone follows the toggle so sound matches reality.
      if (CRG && CRG.state === 'playing') crEngineStart();
    }
  } else {
    crEngineStop();
  }
  crSoundButton();
}
function crSoundButton() {
  const btn = $('#crSoundBtn'); if (!btn) return;
  btn.setAttribute('aria-pressed', String(CRA.on));
  btn.classList.toggle('on', CRA.on);
  const lbl = $('#crSoundLbl');
  if (lbl) lbl.textContent = CRA.on ? 'sound on' : 'sound off';
  btn.title = CRA.on ? 'Sound on — click to mute' : 'Sound off — click to enable';
  btn.setAttribute('aria-label', btn.title);
}


/* =====================================================================
   PHASE 6 · PROGRESSION DATA LAYER
   ---------------------------------------------------------------------
   One localStorage key ('cyber-rush-save-v1'), one schema, validated on
   load. Everything else (achievements, unlocks, records, championship,
   customization) reads/writes through CR_SAVE — no scattered storage.
   ===================================================================== */
const CR_SAVE_KEY = 'cyber-rush-save-v1';

/** The five garage vehicles. Each stat is a MULTIPLIER on the CRP base
 *  physics values — the engine itself is untouched; cars just scale it.
 *  accel        engine pull multiplier
 *  top          max-speed multiplier
 *  steer        steering authority multiplier
 *  grip         lateral friction multiplier (higher = stickier)
 *  drift        drift-grip multiplier (lower = slides more freely)
 *  nitroK       nitro capacity multiplier
 *  body         renderer silhouette family (reuse of Phase 5 shapes) */
const CR_CARS = [
  { id:'aero',   name:'Aero GT',      body:'gt',
    desc:'Balanced flagship — fast everywhere, forgiving nowhere.',
    accel:1.00, top:1.00, steer:1.00, grip:1.00, drift:1.00, nitroK:1.00,
    unlock:null },
  { id:'vortex', name:'Vortex R',     body:'wedge',
    desc:'Low-drag wedge. Highest terminal velocity, twitchy at the limit.',
    accel:0.88, top:1.14, steer:0.94, grip:0.96, drift:1.10, nitroK:1.05,
    unlock:{ type:'distance', value:25, text:'Drive 25 km total' } },
  { id:'kaida',  name:'Kaida Drift',  body:'drift',
    desc:'Set up to slide: huge drift authority, softer straight-line grip.',
    accel:0.96, top:0.94, steer:1.10, grip:0.86, drift:0.62, nitroK:1.00,
    unlock:{ type:'drift', value:400, text:'Bank a 400-point drift' } },
  { id:'brute',  name:'Brute 4X',     body:'muscle',
    desc:'Launch monster. Monster acceleration, heavy nose, lazy steering.',
    accel:1.22, top:0.97, steer:0.86, grip:1.08, drift:1.18, nitroK:0.92,
    unlock:{ type:'nearmiss', value:40, text:'Land 40 near misses' } },
  { id:'phantom',name:'Phantom X',    body:'phantom',
    desc:'Prototype. Best everything — earned, never given.',
    accel:1.10, top:1.07, steer:1.06, grip:1.04, drift:0.80, nitroK:1.15,
    unlock:{ type:'championship', value:1, text:'Win a championship' } },
];

/** Difficulty presets. These scale TRAFFIC and OPPONENT behaviour only —
 *  the player's physics are identical across difficulties (fairness). */
/* P3.4 · ordinary-traffic caps: Easy 6 / Medium 9 / Hard 12. The old
 * values (16/26/36) crowded all three lanes at once, which made the
 * "leave one lane free" rule impossible and produced horizon stacks. */
const CR_DIFFS = {
  easy:   { label:'Easy',   traffic:6,  oppSpeed:0.78, aggression:0.45,
            penaltyK:0.55, speedKeepK:1.25 },
  medium: { label:'Medium', traffic:9,  oppSpeed:1.00, aggression:1.00,
            penaltyK:1.00, speedKeepK:1.00 },
  hard:   { label:'Hard',   traffic:12, oppSpeed:1.24, aggression:1.60,
            penaltyK:1.35, speedKeepK:0.82 },
};

/** Game modes (Phase 6). Each mode changes actual rules:
 *  quick     fixed distance target, standard scoring
 *  timeattack countdown clock, time bonuses for speed/drift/near-miss
 *  endless   no end condition; difficulty ramps with distance          */
const CR_MODES = {
  /* TASK 7.1 · lap-based racing. Quick Race and Championship events are
   * now completed by LAPS, not score thresholds. Default 3 laps; the
   * mode table stays the single source of truth for race shape. */
  quick:      { label:'Quick Race',  laps:3 },
  timeattack: { label:'Time Attack', startMs:75000 },
  endless:    { label:'Endless',     ramp:true },
};

/* =====================================================================
   TASK 7.1 · CHECKPOINT SYSTEM
   The track is a genuine closed circuit (450 segments, height closed on
   the final straight), so checkpoints live in WORLD coordinates: each is
   a segment index on the loop. A lap counts ONLY when every checkpoint is
   crossed in order, forward, exactly once — no skipping, no reversing,
   no duplicate activation. Segment recycling is handled by comparing
   wrapped relative positions, never raw indices.
   ===================================================================== */

/** Build the ordered checkpoint list for a track of `total` segments.
 *  Checkpoints are evenly spaced along the loop with the FINISH LINE at
 *  segment 0 (the start/finish line the player launches from), so a full
 *  lap is one complete circuit crossing every gate forward in order. */
function crMakeCheckpoints(total, count) {
  const cps = [];
  for (let i = 0; i < count; i++) {
    // Finish line first (index 0), then evenly around the loop. The last
    // gap closes back to the finish: spacing is total/count segments.
    cps.push({ seg: Math.round(i * total / count) % total, hit: false });
  }
  return cps;
}
const CR_CHECKPOINTS = 4;          // gates per lap (incl. finish line)

/** Live lap-race state, reset by crNewGame. */
const CR_LAP = {
  enabled: false,          // true only for lap-based modes (quick/champ)
  totalLaps: 3,            // configured lap count
  lap: 1,                  // current lap number, 1-based
  nextCp: 1,               // index of the NEXT checkpoint to cross
  cpFlash: 0,              // seconds of checkpoint HUD flash remaining
  cpFlashText: '',
  lapFlash: 0,             // seconds of lap-complete banner remaining
  lapFlashText: '',
  lapTimes: [],            // completed lap times (ms), in order
  prevLapMs: 0,            // last completed lap
  bestLapMs: 0,            // best lap this race
  finished: false,         // race completed (all laps done)
};

/* =====================================================================
   TASK 7.2 · RACER PARTICIPANTS + REAL-TIME STANDINGS
   A "racer" is a COMPETITOR in the lap race — never ordinary traffic.
   The player is registered as racer #1. The model is deliberately
   self-contained (each racer owns its lap/checkpoint/segment state) so
   Task 7.3 can push AI racers into CR_RACERS WITHOUT touching the
   progress formula or the ranking sort below.
   ===================================================================== */

/** Registered racers for the current race. Index 0 = player. */
const CR_RACERS = [];
/** Standings throttle state (see crStandingsUpdate). */
const CR_STAND = { lastAt: 0 };

/** Build a racer record. `idx` is the registration order and doubles as
 *  the deterministic tie-breaker (stable, never random). */
function crMakeRacer(idx, name, isPlayer) {
  return {
    idx,                        // registration order (tie-break)
    id: isPlayer ? 'player' : 'ai-' + idx,   // unique racer ID
    name,                       // display name
    isPlayer: !!isPlayer,
    lap: 1,                     // current lap (1-based)
    lastCp: 0,                  // last completed checkpoint index this lap
    segF: 0,                    // continuous track segment position (float)
    progress: 0,                // gate-units of race progress (see below)
    finished: false,            // crossed the final finish line
    finishMs: 0,                // race-time stamp of the finish
    pos: 1,                     // current standing (1 = P1)
  };
}

/* =====================================================================
   BUGFIX 3 · AI RACING OPPONENTS (Quick Race)
   Real competitors in the lap race — NOT traffic. Each AI racer reuses
   the existing participant model above (same fields, same progress
   formula via crRacerProgressFrom, same ranking sort) and simulates its
   own ordered-checkpoint lap with the SAME rules as the player: it must
   cross every gate forward, in order, at its own speed. Count scales
   with difficulty. They are rendered as racing rivals on the road and
   feed AHD/BHD gaps + standings — never counted as traffic.
   ===================================================================== */
const CR_AI_NAMES = ['VEGA', 'ORION', 'LYRA', 'NOVA-9', 'KAIJU', 'SABLE',
                     'RUNE', 'HALCYON'];
/** P3 · exactly FIVE race participants on every difficulty: the player
 *  plus four AI rivals. Difficulty changes how strong the rivals are,
 *  never how many cars are on the grid. Ordinary traffic is NOT counted
 *  — it is never registered in CR_RACERS. */
const CR_AI_COUNT = { easy: 4, medium: 4, hard: 4 };

/** Register the field for a new race. Lap modes get the player plus AI
 *  opponents (BUGFIX 3); other modes get an EMPTY field — Time Attack
 *  and Endless are not multi-racer races and must never fabricate one. */
function crRacersReset(mode, diff) {
  CR_RACERS.length = 0;
  CR_STAND.lastAt = 0;
  const totalSegs = CRG.totalSegs || 450;   // P3.14 · grid maths needs the loop length
  if (mode === 'quick') {
    CR_RACERS.push(crMakeRacer(0, 'YOU', true));
    const n = CR_AI_COUNT[diff || CRG.diff || 'medium'] || 0;
    for (let i = 0; i < n; i++) {
      const r = crMakeRacer(i + 1, CR_AI_NAMES[i % CR_AI_NAMES.length], false);
      /* Skill band: opponents cruise at a fraction of the player's rated
       * cap, spread so the field strings out naturally (a leader to chase,
       * a tail to defend against). Faster difficulties raise the band.
       * P4.4 · rebalanced so P1 is achievable with good driving on
       * Medium: the strongest rival now cruises just under a well-driven
       * pace (~95% cap held cleanly), and the band widens downward so a
       * clean race with nitro use beats the whole field. Difficulty still
       * raises/lower the band; it no longer pushes rivals past the
       * player's realistic ceiling. */
      const D = CR_DIFFS[diff || CRG.diff || 'medium'] || CR_DIFFS.medium;
      /* P4.4 · final tuning: the leader cruises at ~84% of the rated cap
       * on Medium. A clean human line (95%+ with nitro, losing a little
       * in corners) laps ~39s vs the leader's ~43s — P1 is earned with
       * good driving, lost with mistakes. Easy/Hard shift the whole band. */
      const bandTop = 0.72 + D.oppSpeed * 0.12;      // easy .814 / med .84 / hard .869
      const bandSpan = 0.26;                         // leader .. tail spread
      const skill = Math.min(0.88, Math.max(0.50,
        bandTop - (i / Math.max(1, n - 1 || 1)) * bandSpan));
      r.ai = {
        speedK: skill,                               // cruise fraction of maxSpeed
        segF: 0,                                     // continuous segment position
        nextCp: 1,                                   // next gate index (ordered)
        color: CR_CAR_COLORS[(i + 1) % CR_CAR_COLORS.length],
        style: 'sport',                              // low wedge — reads as a rival
        laneT: 0,                                    // lane-change timer
        lane: (i % CR.LANES),                        // start lane (spread)
        x: 0,                                        // lateral position (-1..1)
      };
      /* P3.14 · proper starting grid: a staggered TWO-COLUMN arrangement
       * across different lanes. Column A (even i) sits on the left half
       * of the road, column B (odd i) on the right, and each row is one
       * grid slot further back so nobody starts inside anybody else.
       * The player occupies the front-row centre (segment 0, lane 1).
       * gridOffset records how far behind the line this rival starts so
       * the finish test can compare DISTANCE RACED, not raw position. */
      const colA = (i % 2 === 0);
      const gridRow = Math.floor(i / 2) + 1;         // 1,1,2,2,3,3...
      r.ai.lane = colA ? 0 : (CR.LANES - 1);         // outer columns only
      const back = gridRow * 3.2;                    // segments behind the line
      r.ai.gridOffset = back;                        // distance already "owed"
      /* P3.14c · segF stays NEGATIVE here (do NOT wrap onto the loop):
       * wrapping added a full lap to every back-row rival's raw segF,
       * which inflated "distance raced" by totalSegs and made them finish
       * a lap early. A negative segF is safe — every consumer either
       * wraps it itself (%totalSegs) or adds gridOffset first. */
      r.ai.segF = -back;
      r.ai.x = -1 + (2 / CR.LANES) * (r.ai.lane + 0.5);
      r.segF = ((r.ai.segF % totalSegs) + totalSegs) % totalSegs;
      CR_RACERS.push(r);
    }
  }
}

/** Advance one AI racer for dt seconds using the SAME ordered-checkpoint
 *  discipline as the player: continuous position advances only forward,
 *  gates register when crossed in order, and progress comes from the
 *  shared formula. Pure simulation — no rendering, no physics coupling. */
function crAiStep(r, dt) {
  const g = CRG, L = CR_LAP;
  if (!r.ai || r.finished) return;
  const total = g.totalSegs;
  const cps = g.cps;
  const n = cps.length;

  /* P3.11 · rubber-band capped at ±10%. The old "breathing" sine alone
   * was fine, but any future catch-up logic must stay inside a hard
   * band: rivals never exceed 110% nor drop below 90% of their own
   * baseline cruise (skill × rated cap). The band is applied to the
   * FINAL speed so nothing downstream can re-inflate it. */
  const baseSpd = g.maxSpeed * r.ai.speedK;
  const breathe = 1 + Math.sin(performance.now() / 1700 + r.idx * 2.1) * 0.05;
  let spd = baseSpd * breathe;
  spd = Math.max(baseSpd * 0.90, Math.min(baseSpd * 1.10, spd));
  let prev = r.ai.segF;
  r.ai.segF += (spd / CR.SEG_LEN) * dt;
  /* P3.14b · finish test on DISTANCE RACED, not raw loop position: grid
   * slots sit behind the start line, so a rival's segF begins near the
   * end of the loop. Comparing raw segF against total*totalLaps made
   * back-row rivals "finish" a full lap early. Distance raced =
   * segF + gridOffset (the owed grid deficit). */
  const raced = r.ai.segF + (r.ai.gridOffset || 0);
  if (raced >= total * L.totalLaps + 0.001 && !r.finished) {
    // clamp at the finish so nobody over-runs the line
    r.ai.segF = total * L.totalLaps - (r.ai.gridOffset || 0);
  }

  // Ordered gate crossings (integer segment boundaries, forward only).
  const prevIdx = ((Math.floor(prev) % total) + total) % total;
  const newIdxAbs = Math.floor(r.ai.segF);
  const newIdx = ((newIdxAbs % total) + total) % total;
  let moved = crWrapSeg(newIdx - prevIdx, total);
  if (moved < 0) moved = 0;
  while (moved > 0) {
    const cp = cps[r.ai.nextCp];
    if (!cp) break;
    let rel = crWrapSeg(cp.seg - prevIdx, total);
    if (rel <= 0) rel += total;
    if (rel > moved) break;                          // not reached yet
    // Gate crossed: advance the machine exactly like the player's.
    r.ai.nextCp = (r.ai.nextCp + 1) % n;
    moved -= rel;
    if (r.ai.nextCp === 1) {                         // wrapped past finish
      r.lap++;
      if (r.lap > L.totalLaps) {
        r.finished = true;
        r.finishMs = g.raceTimeMs;
        r.lap = L.totalLaps;
        break;
      }
    }
  }

  /* P3.12 · traffic-aware lane changes. A rival scans ordinary traffic
   * ahead in its lane; when a slower car blocks the way it changes to an
   * adjacent lane (never teleporting — x eases toward the new centre).
   * With nothing ahead it drifts occasionally, as before, so the field
   * still spreads naturally. */
  const laneW = 2 / CR.LANES;
  const mySegF = ((r.ai.segF % total) + total) % total;
  let blockedAhead = false;
  for (const t of g.traffic) {
    let rz = t.z - mySegF;
    if (rz < -total / 2) rz += total;
    if (rz > total / 2) rz -= total;
    if (rz > 0.5 && rz < 14 && Math.abs(t.x - r.ai.x) < laneW * 0.8) {
      blockedAhead = true; break;
    }
  }
  r.ai.laneT += dt;
  const wantChange = blockedAhead
    || (r.ai.laneT > 2.2 + r.idx * 0.3);
  if (wantChange) {
    r.ai.laneT = 0;
    if (blockedAhead) {
      // Pick the adjacent lane least occupied by traffic.
      const laneOccupied = l => {
        let cnt = 0;
        const lx = -1 + laneW * (l + 0.5);
        for (const t of g.traffic) {
          let rz = t.z - mySegF;
          if (rz < -total / 2) rz += total;
          if (rz > total / 2) rz -= total;
          if (rz > -6 && rz < 20 && Math.abs(t.x - lx) < laneW * 0.8) cnt++;
        }
        return cnt;
      };
      const opts = [];
      if (r.ai.lane > 0) opts.push(r.ai.lane - 1);
      if (r.ai.lane < CR.LANES - 1) opts.push(r.ai.lane + 1);
      let best = null, bestCnt = Infinity;
      for (const l of opts) {
        const cnt = laneOccupied(l);
        if (cnt < bestCnt) { bestCnt = cnt; best = l; }
      }
      if (best != null) r.ai.lane = best;
    } else {
      r.ai.lane = Math.max(0, Math.min(CR.LANES - 1,
        r.ai.lane + (Math.random() < 0.5 ? -1 : 1)));
    }
  }
  const targetX = -1 + laneW * (r.ai.lane + 0.5);
  r.ai.x += (targetX - r.ai.x) * Math.min(1, dt * 1.4);

  // Shared bookkeeping for ranking + rendering.
  r.segF = ((r.ai.segF % total) + total) % total;
  /* P3.14b · lap/gate accounting also uses distance raced so a back-row
   * rival does not get credit for the grid deficit it has not covered. */
  const raced2 = r.ai.segF + (r.ai.gridOffset || 0);
  let lapDone = Math.min(L.totalLaps, Math.floor(raced2 / total));
  if (lapDone >= L.totalLaps) {                    // covered every lap
    r.finished = true;
    if (!r.finishMs) r.finishMs = g.raceTimeMs;
    r.lap = L.totalLaps;
    lapDone = L.totalLaps;
  }
  const gatesHit = r.finished ? n : r.ai.nextCp === 0 ? n - 1 : Math.max(0, r.ai.nextCp - 1);
  const nextGate = cps[r.finished ? 0 : r.ai.nextCp].seg;
  /* Progress is hard-capped at the race total: a finished racer has
   * covered exactly totalLaps × gatesPerLap gate-units, never more. */
  r.progress = Math.min(L.totalLaps * n, crRacerProgressFrom(
    r.finished ? L.totalLaps : lapDone, gatesHit,
    ((r.ai.segF % total) + total) % total, nextGate, n, total));
}

/** Race progress in GATE-UNITS: one unit = advancing past one checkpoint
 *  gate. progress = completedLaps * gatesPerLap + gatesHitThisLap +
 *  fraction toward the next gate. Because gate hits can only come from
 *  the ordered checkpoint machine, this can never be inflated by raw
 *  distance, reversing or recycling — the failure modes of using
 *  g.position alone. Within a gate span the fraction is continuous, so
 *  progress is continuous across checkpoint AND lap transitions.
 *
 *  NOTE: the player's values are READ from the live game state (CR_LAP /
 *  CRG) — no second bookkeeping to drift out of sync. AI racers (7.3)
 *  will carry their own lap/lastCp/segF and call the same maths via
 *  crRacerProgressFrom(). */
function crRacerProgressFrom(lapDone, gatesHit, segF, nextGateSeg, gatesPerLap, totalSegs) {
  let frac = 0;
  if (lapDone >= 0 && nextGateSeg != null) {
    let d = crWrapSeg(nextGateSeg - segF, totalSegs);
    if (d <= 0) d += totalSegs;                 // gate is always ahead
    const span = totalSegs / gatesPerLap;
    frac = Math.max(0, Math.min(1, 1 - d / span));
  }
  return lapDone * gatesPerLap + gatesHit + frac;
}

/** Player's live progress, derived entirely from authoritative state. */
function crPlayerProgress() {
  const g = CRG, L = CR_LAP;
  const n = g.cps ? g.cps.length : CR_CHECKPOINTS;
  if (L.finished) return L.totalLaps * n;       // full distance, exactly
  const lapDone = Math.max(0, L.lap - 1);
  // Gates crossed this lap: nextCp counts from 1; when it wraps to 0 all
  // non-finish gates are done.
  const gatesHit = L.nextCp === 0 ? (n - 1) : (L.nextCp - 1);
  const segF = g.position / CR.SEG_LEN;
  const nextGate = g.cps ? g.cps[L.nextCp].seg : 0;
  /* P3.14b · hard cap at the race total (mirrors the AI path): progress
   * can never exceed totalLaps × gatesPerLap even for a frame where the
   * finish crossing and the standings refresh race each other. */
  return Math.min(L.totalLaps * n, crRacerProgressFrom(
    lapDone, gatesHit, segF, nextGate, n, g.totalSegs));
}

/** Recompute standings. Called from crStep — forced on checkpoint/lap/
 *  finish events, otherwise throttled to ~7 Hz so the render loop never
 *  pays for a sort it doesn't need (progress moves smoothly between
 *  gates; 150 ms staleness is invisible next to gate granularity).
 *
 *  Ranking rules (deterministic):
 *    1. Finished racers first, ordered by finish timestamp (earlier wins)
 *       — a finished racer can never be overtaken.
 *    2. Then unfinished racers by progress (descending).
 *    3. Ties break by registration index (never random, never unstable). */
function crStandingsUpdate(force) {
  const g = CRG, L = CR_LAP;
  if (!L.enabled || !CR_RACERS.length) return;
  const now = performance.now();
  if (!force && now - CR_STAND.lastAt < 150) return;
  CR_STAND.lastAt = now;

  for (const r of CR_RACERS) {
    if (r.isPlayer) {
      r.segF = g.position / CR.SEG_LEN;
      r.lap = L.finished ? L.totalLaps : L.lap;
      // Gates crossed this lap (finish gate wraps nextCp to 0).
      const nGates = g.cps.length;
      r.lastCp = L.finished ? 0
               : (L.nextCp === 0 ? nGates - 1 : Math.max(0, L.nextCp - 1));
      r.progress = crPlayerProgress();
      if (L.finished && !r.finished) {
        r.finished = true;                      // capture ONCE
        r.finishMs = g.raceTimeMs;
      }
    } else if (r.ai) {
      /* BUGFIX 3 · AI racers simulate in the physics tick (crAiStep is
       * called from crStep), so by the time we sort, their lap/gates/
       * progress fields are already current. Nothing to recompute here —
       * just keep lastCp aligned with their machine for display. */
      r.lastCp = r.ai.nextCp === 0 ? g.cps.length - 1 : Math.max(0, r.ai.nextCp - 1);
    }
  }

  CR_RACERS.sort((a, b) => {
    if (a.finished && b.finished)
      return (a.finishMs - b.finishMs) || (a.idx - b.idx);
    if (a.finished) return -1;
    if (b.finished) return 1;
    return (b.progress - a.progress) || (a.idx - b.idx);
  });
  for (let i = 0; i < CR_RACERS.length; i++) CR_RACERS[i].pos = i + 1;
}

/** Wrap a raw segment delta into the shortest signed distance on the
 *  loop (-total/2 .. total/2). All checkpoint comparisons use this, so
 *  recycled indices can never fake or hide a crossing. */
function crWrapSeg(dz, total) {
  if (dz < -total / 2) dz += total;
  if (dz > total / 2) dz -= total;
  return dz;
}

/** Advance the checkpoint state machine for this frame.
 *  Called from crStep AFTER position advances, with the player's PREVIOUS
 *  segment index. Crossing detection is directional: a checkpoint only
 *  activates when the player moves FORWARD across its segment boundary
 *  while it is the next expected one. Physics is untouched — this is a
 *  pure observer of position. Returns 'checkpoint' | 'lap' | 'finish' |
 *  null. */
function crCheckpointStep(prevSegIdx, newSegIdx) {
  const g = CRG, L = CR_LAP;
  if (!L.enabled || L.finished || !g.cps || g.cps.length < 2) return null;
  const total = g.totalSegs;
  // Forward motion this tick (wrapped): reverse driving gives negative.
  const moved = crWrapSeg(newSegIdx - prevSegIdx, total);
  if (moved <= 0) return null;                       // stationary/reversing

  /* BUGFIX 1 · physical plausibility cap. Physics can advance at most
   * nitroSpeed * dtMax world units per tick (dt is clamped to 0.05 s),
   * i.e. ~4 segments. Any larger jump is a teleport or a corrupted index,
   * never real driving — reject it outright so a stray state write can
   * never fire a gate (and eventually a lap) without covering distance. */
  const maxSegsPerTick = Math.ceil((g.nitroSpeed || g.maxSpeed) * 0.05 / CR.SEG_LEN) + 1;
  if (moved > maxSegsPerTick) return null;

  // Only the EXPECTED checkpoint can activate — order is enforced by
  // construction. Duplicate crossings are impossible because a hit flag
  // clears only on lap completion; but we still verify the player is
  // actually AT that checkpoint's segment (no teleport/skip shortcuts).
  const cp = g.cps[L.nextCp];
  if (!cp || cp.hit) return null;

  // Did we cross cp.seg going forward this tick? Check the half-open arc
  // (prevSegIdx, newSegIdx] on the loop — a step of `moved` segments.
  let rel = crWrapSeg(cp.seg - prevSegIdx, total);
  if (rel <= 0 || rel > moved) return null;          // not crossed yet

  const wasFinishGate = (L.nextCp === 0);            // finish line == lap end
  /* BUGFIX 1 · full-circuit assertion. The finish gate may only complete
   * a lap when EVERY non-finish gate was already crossed this lap in
   * order. nextCp===0 already implies that by construction, but a
   * hand-edited or restored state could claim otherwise — verify against
   * the actual hit flags so a lap can NEVER complete early. */
  if (wasFinishGate) {
    for (let i = 1; i < g.cps.length; i++) {
      if (!g.cps[i].hit) return null;                // incomplete circuit
    }
  }

  cp.hit = true;
  const last = wasFinishGate;
  L.nextCp = (L.nextCp + 1) % g.cps.length;

  if (!last) {
    L.cpFlash = 1.1;
    /* BUGFIX 2 · the counter must show the gate JUST CROSSED, not the
     * post-increment pointer. Gate indices 1..n-1 map to displays
     * 1/3..3/3; the finish gate (index 0) never prints a "0/3" counter. */
    L.cpFlashText = `CHECKPOINT ${(L.nextCp === 0 ? g.cps.length - 1 : L.nextCp - 1)}/${g.cps.length - 1}`;
    crPopup('CHECKPOINT', '#40e0ff');
    crSfx('pass');
    return 'checkpoint';
  }
  /* BUGFIX 2 · clear any stale checkpoint text the moment the finish line
   * is crossed — the lap banner takes over; no lingering "x/3" readout. */
  L.cpFlash = 0; L.cpFlashText = '';

  // ---- finish line crossed: a full ordered circuit is complete ----
  const lapMs = g.raceTimeMs - g.lapStart;
  g.lapStart = g.raceTimeMs;
  L.lapTimes.push(lapMs);
  L.prevLapMs = lapMs;
  if (!L.bestLapMs || lapMs < L.bestLapMs) L.bestLapMs = lapMs;
  if (!g.bestLapMs || lapMs < g.bestLapMs) g.bestLapMs = lapMs;
  // Reset all gates for the next lap (hit flags + order pointer).
  for (const c of g.cps) c.hit = false;

  if (L.lap >= L.totalLaps) {
    // RACE COMPLETE — freeze progression; crFinish handles the rest.
    g.laps++;                                // count the final lap too
    L.finished = true;
    L.lapFlash = 2.2;
    L.lapFlashText = 'FINISH!';
    crPopup('FINISH!', '#5ef2c0');
    crSfx('win');
    return 'finish';
  }
  L.lap++;
  g.laps++;                                   // legacy counter stays truthful
  const finalLap = L.lap === L.totalLaps;
  L.lapFlash = 2.2;
  L.lapFlashText = finalLap ? 'FINAL LAP' : `LAP ${L.lap}/${L.totalLaps}`;
  crPopup(finalLap ? 'FINAL LAP' : `LAP ${L.lap}`, finalLap ? '#ffc857' : '#e8ecf4');
  crSfx(finalLap ? 'unlock' : 'go');
  return 'lap';
}

/** Achievements. `test(run, save)` receives the finished run summary and
 *  the persistent totals; returning true awards it (once — duplicates
 *  are blocked by the stored set). */
const CR_ACHIEVEMENTS = [
  { id:'first',   name:'First Race',     icon:'🏁',
    text:'Finish your first race.',
    test:(r)=>r.races>=1 },
  { id:'speed',   name:'Speed Demon',    icon:'⚡',
    text:'Sustain over 300 km/h.',
    test:(r)=>r.maxKmh>=300 },
  { id:'drifter', name:'Drift Master',   icon:'🌀',
    text:'Bank 1000 drift points in one race.',
    test:(r)=>r.driftTotal>=1000 },
  { id:'near',    name:'Near Miss Expert',icon:'💨',
    text:'20 near misses in one race.',
    test:(r)=>r.nearMisses>=20 },
  { id:'clean',   name:'Clean Driver',   icon:'✨',
    text:'Finish a race of 2+ km without a crash.',
    test:(r)=>r.crashes===0 && r.distanceKm>=2 },
  { id:'nitro',   name:'Nitro Addict',   icon:'🔥',
    text:'Use nitro for 30 seconds in one race.',
    test:(r)=>r.nitroUsedS>=30 },
  { id:'dist',    name:'Distance Champion',icon:'🛣️',
    text:'Cover 50 km in total.',
    test:(r,s)=>s.totals.distanceKm>=50 },
  { id:'endless', name:'Endless Survivor',icon:'♾️',
    text:'Survive 3 km in Endless mode.',
    test:(r)=>r.mode==='endless' && r.distanceKm>=3 },
];

/** Neon accent palettes (customization). Body colours are free; neon and
 *  wheels/spoilers are partly unlock-gated to give progression texture. */
const CR_NEONS = ['#5ef2c0','#40e0ff','#b48cff','#ff5ca8','#ffc857','#ff6b5e'];
const CR_WHEELS = [
  { id:'stock', name:'Stock',   rim:'#2b3345', spoke:'#8b96ad' },
  { id:'sport', name:'Sport',   rim:'#3d4b66', spoke:'#c3cede' },
  { id:'carbon',name:'Carbon',  rim:'#14181f', spoke:'#5a6c8a', unlock:{type:'races',value:5,text:'Finish 5 races'} },
  { id:'gold',  name:'Gold',    rim:'#6b5a2e', spoke:'#e8c96a', unlock:{type:'races',value:15,text:'Finish 15 races'} },
];
const CR_SPOILERS = [
  { id:'duck',  name:'Ducktail', h:0.06 },
  { id:'gt',    name:'GT Wing',  h:0.12 },
  { id:'none',  name:'Clean',    h:0 },
  { id:'twin',  name:'Twin Strut',h:0.10, unlock:{type:'drift',value:800,text:'Bank an 800-point drift'} },
];

/** Default persisted state. */
function crSaveDefault() {
  return {
    v: 1,
    car: 'aero',
    custom: { body:'#49e0b0', neon:'#5ef2c0', wheel:'stock', spoiler:'gt', plate:'SYN-01' },
    records: {
      highScore: 0, longestKm: 0, topKmh: 0, bestDrift: 0,
      mostNear: 0, cleanestKm: 0, bestChampPts: 0,
      bestLapMs: 0,                        // TASK 7.1 · all-time best lap
    },
    totals: {
      races: 0, distanceKm: 0, driftTotal: 0, nearMisses: 0,
      nitroUsedS: 0, crashes: 0, wins: 0,
    },
    achievements: [],                       // awarded ids
    champ: null,                            // active championship
    champHistory: [],                       // completed championships
  };
}

/** Validate + migrate a loaded save. Anything malformed falls back to the
 *  default for THAT FIELD ONLY, so one bad key can't poison the rest. */
function crSaveValidate(raw) {
  const def = crSaveDefault();
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return def;
  const out = def;
  if (raw.v === 1) {
    if (typeof raw.car === 'string' && CR_CARS.some(c=>c.id===raw.car)) out.car = raw.car;
    if (raw.custom && typeof raw.custom === 'object') {
      const c = raw.custom;
      if (/^#[0-9a-f]{6}$/i.test(c.body||''))   out.custom.body = c.body;
      if (/^#[0-9a-f]{6}$/i.test(c.neon||''))   out.custom.neon = c.neon;
      if (CR_WHEELS.some(w=>w.id===c.wheel))    out.custom.wheel = c.wheel;
      if (CR_SPOILERS.some(s=>s.id===c.spoiler))out.custom.spoiler = c.spoiler;
      if (typeof c.plate === 'string' && c.plate.length<=8) out.custom.plate = c.plate.slice(0,8);
    }
    /* Per-field validation with the correct fallback table: records fall
     * back to default records, totals to default totals. (A shared
     * fallback silently DROPPED total fields when one was malformed.) */
    const num = (o,k,fb)=>{ const v=o&&o[k];
      return typeof v==='number' && isFinite(v) && v>=0 ? v : fb; };
    if (raw.records && typeof raw.records === 'object') {
      for (const k of Object.keys(def.records))
        out.records[k] = num(raw.records, k, def.records[k]);
    }
    if (raw.totals && typeof raw.totals === 'object') {
      for (const k of Object.keys(def.totals))
        out.totals[k] = num(raw.totals, k, def.totals[k]);
    }
    if (Array.isArray(raw.achievements)) {
      out.achievements = raw.achievements.filter(a=>typeof a==='string'
        && CR_ACHIEVEMENTS.some(x=>x.id===a));
    }
    if (raw.champHistory && Array.isArray(raw.champHistory)) {
      out.champHistory = raw.champHistory.slice(0,20).filter(h=>h&&typeof h==='object'
        && typeof h.name==='string' && Array.isArray(h.standings));
    }
    if (raw.champ && typeof raw.champ === 'object' && Array.isArray(raw.champ.events)) {
      out.champ = crChampValidate(raw.champ);
    }
  }
  return out;
}

/** Championship definition: 4 events at escalating difficulty. Points are
 *  awarded per event from the race score/rating — nothing fabricated. */
const CR_CHAMP_EVENTS = [
  { name:'Neon Sprint',  mode:'quick',      diff:'easy',   target:4000 },
  { name:'Harbor Loop',  mode:'timeattack', diff:'medium', target:6000 },
  { name:'Summit Rush',  mode:'quick',      diff:'medium', target:8000 },
  { name:'Final Circuit',mode:'quick',      diff:'hard',   target:10000 },
];
function crChampPoints(result) {
  // Score-based, scaled by rating letter (D..S -> 0.6..1.5).
  const K = { D:0.6, C:0.8, B:1.0, A:1.25, S:1.5 };
  return Math.round((result.score / 40) * (K[result.rating] || 1));
}
function crChampNew() {
  return {
    startedAt: Date.now(),
    eventIdx: 0,                              // next event to run
    points: 0,                                // player cumulative
    events: CR_CHAMP_EVENTS.map(e => ({ ...e, done:false, result:null })),
  };
}
function crChampValidate(ch) {
  const fresh = crChampNew();
  if (typeof ch.eventIdx !== 'number' || ch.eventIdx < 0 || ch.eventIdx > fresh.events.length) return null;
  if (!Array.isArray(ch.events) || ch.events.length !== fresh.events.length) return null;
  // Rebuild from the trusted template; keep only per-event results.
  const out = fresh;
  out.eventIdx = Math.floor(ch.eventIdx);
  out.points = (typeof ch.points === 'number' && isFinite(ch.points) && ch.points >= 0)
    ? ch.points : 0;
  out.startedAt = (typeof ch.startedAt === 'number') ? ch.startedAt : Date.now();
  for (let i = 0; i < out.events.length; i++) {
    const r = ch.events[i] && ch.events[i].result;
    if (i < out.eventIdx && r && typeof r === 'object') {
      out.events[i].done = true;
      out.events[i].result = {
        score: Math.max(0, Math.round(+r.score || 0)),
        rating: ['D','C','B','A','S'].includes(r.rating) ? r.rating : 'D',
        won: !!r.won,
      };
    }
  }
  return out;
}

/** Load (validated) or reset the save. */
function crSaveLoad() {
  try {
    const raw = localStorage.getItem(CR_SAVE_KEY);
    if (!raw) return crSaveDefault();
    return crSaveValidate(JSON.parse(raw));
  } catch { return crSaveDefault(); }        // quota/corrupt: start fresh
}
function crSaveWrite() {
  try { localStorage.setItem(CR_SAVE_KEY, JSON.stringify(CR_SAVE)); }
  catch { /* private mode / quota: play on without persistence */ }
}
let CR_SAVE = crSaveLoad();

/** Is a vehicle/custom item unlocked? (null unlock = free.) */
function crUnlocked(item, save) {
  if (!item || !item.unlock) return true;
  const s = save || CR_SAVE;
  const t = s.totals, r = s.records;
  switch (item.unlock.type) {
    case 'distance':  return t.distanceKm >= item.unlock.value;
    case 'drift':     return r.bestDrift  >= item.unlock.value;
    case 'nearmiss':  return t.nearMisses >= item.unlock.value;
    case 'races':     return t.races      >= item.unlock.value;
    case 'championship':
      return s.champHistory.some(h => h.wonByPlayer === true);
    default: return false;
  }
}

/* ---- 2. Track + world constants -------------------------------------- */
const CR = {
  W: 960, H: 540,           // logical canvas units (all drawing uses these)
  /* FULLSCREEN/VIEWPORT FIX · live logical view size. CR.W/CR.H stay the
   * canonical design units (physics, projection and tests read them);
   * crResize() updates CR.vW/cr.vH to the ACTUAL stage aspect so the
   * rendered world fills any viewport without stretching or letterbox
   * bands. vW is clamped to a sane band around the design width so an
   * ultra-wide monitor shows more city/verge (never a squashed road) and
   * a portrait phone keeps the full road visible. Drawing code reads
   * crView() instead of the raw constants. */
  vW: 960, vH: 540,
  VW_MIN: 720,              // portrait floor: road always fully visible
  VW_MAX: 1720,             // ultrawide ceiling: HUD instruments stay readable
  SEG_LEN: 200,             // world length of one road segment
  ROAD_W: 2200,             // half-width of the road in world units
  LANES: 3,                 // traffic lanes
  DRAW_DIST: 150,           // segments drawn ahead of the camera
  FOV: 100,                 // degrees at rest (widens with speed)
  /* TASK 7.0 · raised so the player's plane (with the trail below) sits
   * low on screen while the road at that row spans ~1.4x the canvas —
   * outer-lane neighbours stay visible at the frame edge instead of
   * exiting the view entirely. One value for road + traffic + player. */
  CAM_H: 1435,              // camera height above the road
  /* TASK 7.0 · chase-camera geometry. The camera trails the player's own
   * world position by CAM_TRAIL units, so the player's z projects onto a
   * fixed screen row (~H-32) exactly like every traffic car — one
   * projection model for road, traffic AND player. A car therefore
   * REACHES the player's visual plane precisely when rel≈0, which is
   * where the collision z-window already sits: no more pass-throughs. */
  CAM_TRAIL: 1300,          // camera sits 6.5 segments behind the player
  /* TASK 7.0 · uniform vehicle visual scale. Road-unit widths are large
   * relative to the camera, so true-unit rendering makes adjacent cars
   * dwarf the fixed-size player sprite. CAR_VIS_K scales ALL traffic
   * bodies by the same factor, keeping perspective consistent between
   * vehicles while matching the player's arcade proportion. */
  CAR_VIS_K: 0.62,
  /* BUGFIX (scenery/wheels) · two constants were referenced by the
   * renderer but never declared, silently poisoning three systems:
   *
   * WHEEL_R — wheel radius in world units. g.wheelSpin accumulated
   *   speed/undefined = NaN, so the player's wheel spokes never drew
   *   (cos(NaN) drops the path). Radius ~34u matches the drawn sprite.
   *
   * RAIL_H — guardrail height in world units. Every rail-top projection
   *   was seg.y + undefined = NaN, so the guardrail FACES and neon rail
   *   tops never rendered at all — the road ran beside an invisible
   *   barrier. ~85u reads as a waist-high motorway barrier at CAM_H. */
  WHEEL_R: 34,
  RAIL_H: 85,
};

/* =====================================================================
   PHYSICS CONFIGURATION — every tunable in ONE place.
   Units: speeds are world-units/second, forces are world-units/second²,
   grip values are exponential-decay rates per second (frame-rate safe).
   Rendering code must read from here, never re-declare its own numbers.
   ===================================================================== */
const CRP = {
  // --- longitudinal ---------------------------------------------------
  MAX_SPEED:      12000,   // base cap (world u/s); timeattack overrides
  ACCELERATION:   5400,    // peak engine pull at low revs
  NITRO_ACCEL:    9200,    // engine pull while boosting
  TORQUE_FALLOFF: 0.65,    // fraction of accel lost at the cap
  BRAKE_FORCE:    11200,   // service braking deceleration
  ENGINE_BRAKE:   2400,    // coasting deceleration
  OVERSPILL_BLEED:6800,    // how fast excess speed bleeds after nitro ends
  OFFROAD_DRAG:   5200,    // grass drag at the verge edge
  OFFROAD_DRAG_K: 4200,    // extra drag per unit further off the road
  OFFROAD_MIN_FR: 0.32,    // floor (fraction of max speed) on grass

  // --- steering -------------------------------------------------------
  STEERING_RESPONSE: 7.5,  // input ramp rate (per second) — inertia
  STEER_AUTH_BASE:  1.85,  // peak lateral authority
  STEER_AUTH_LO:     0.34, // authority fraction at standstill
  STEER_AUTH_RAMP:   1.7,  // speed multiple at which authority saturates
  STEER_SPEED_MIX:   0.55, // low-speed share of the steer force curve

  // --- lateral / grip / drift ----------------------------------------
  LATERAL_FRICTION: 4.6,   // base tyre grip (decay rate /s)
  GRIP_SPEED_LOSS:  2.1,   // grip lost at top speed (quadratic)
  DRIFT_GRIP:       0.52,  // grip multiplier while drift is HELD
  DRIFT_GRIP_RECOVER: 0.72,// grip multiplier while still sliding after release
  /* NATURAL STEERING FIX · extra tyre bite when the wheel is released and
   * no slide is in progress — kills the floaty post-release drift so the
   * car settles where the input stops. 1.6 ≈ lateral velocity halves
   * every ~90ms on release. */
  GRIP_RELEASE_BITE: 1.6,  // grip multiplier with no input & no slide
  GRASS_GRIP:       0.42,  // grip multiplier on the verge
  VLAT_SNAP:        0.004, // residual lateral velocity snapped to zero
  DRIFT_SLIP_MIN:   0.62,  // lateral slip that counts as a drift
  DRIFT_SPEED_MIN:  0.30,  // speed fraction required to drift at all
  DRIFT_MIN_TIME:   0.55,  // seconds before a drift can bank points
  DRIFT_BANK_MIN:   40,    // smallest banked payout
  DRIFT_HOLD_STEER: 0.18,  // steering input needed to KEEP drifting
  CURVE_PULL:       0.115, // centrifugal push toward the outside of bends

  // --- road boundaries -------------------------------------------------
  ROAD_HALF:       1.0,    // |playerX| at the tarmac edge
  BOUND_MAX:       1.38,   // hard clamp (barrier)
  SELF_CENTRE:     0.55,   // verge pull back toward tarmac
  SELF_CENTRE_K:   1.1,    // verge pull growth per unit off-road
  EDGE_EASE:       0.12,   // distance over which the verge pull eases in

  // --- nitro ------------------------------------------------------------
  NITRO_MULTIPLIER: 1.32,  // top-speed multiple while boosting
  NITRO_CAPACITY:   3.2,   // seconds of burn from a full meter
  NITRO_REGEN_FAST: 0.055, // meter refill/s while driving fast
  NITRO_REGEN_SLOW: 0.02,  // meter refill/s otherwise
  /* BUGFIX 4 · skill-based nitro charges. A clean overtake adds a small
   * chunk; threading a near miss adds more (risk -> reward). Values are
   * fractions of the full meter per event, awarded once per approach. */
  NITRO_GAIN_PASS: 0.04,   // plain overtake
  NITRO_GAIN_MISS: 0.09,   // overtake with a near miss
  NITRO_MIN_SPEED:  0.35,  // speed fraction below which nitro refuses
  NITRO_MIN_ENERGY: 0.001, // meter floor required to engage
  NITRO_SHAKE:      0.06,  // camera shake amplitude during boost

  // --- collision ---------------------------------------------------------
  COLLISION_PENALTY_HEAD: 90,   // score lost, head-on
  COLLISION_PENALTY_SIDE: 45,   // score lost, side swipe
  COLLISION_SPEED_HEAD:   0.30, // speed kept after a head-on hit
  COLLISION_SPEED_SIDE:   0.62, // speed kept after a glancing blow
  COLLISION_GRACE:        1.1,  // invulnerability window (seconds)
  COLLISION_GRACE_HOLD:   0.35, // min remaining grace while scraping
  COLLISION_SLOWMO_T:     0.18, // impact slow-motion duration (seconds)
  COLLISION_SLOWMO_K:     0.42, // time scale during impact slow-motion
  PLAYER_WIDTH:           0.56, // player car width in road units
  /* BUGFIX 6 · collision half-widths must match the DRAWN widths. Both
   * traffic and the player are rendered through the same chain
   * (p.w * 0.56 * bodyW * CAR_VIS_K), so the physical contact widths use
   * the identical factors — a visible overlap is now always a real hit,
   * and a visible gap is never a phantom one. */
  CAR_BODY_K:              0.56, // body width factor (shared render chain)

  // --- camera -------------------------------------------------------------
  CAM_LAG:        6.5,     // chase-camera lateral damping (/s)
  CAM_LAG_STAB:   11.0,    // tightened lag right after a collision (/s)
  CAM_DRIFT_K:    1.14,    // drift widens the follow (leans out of slides)
  CAM_INSIDE:     0.82,    // camera target fraction of playerX
  CAM_H_LAG:      4.0,     // height-follow damping (/s)
  FOV_SPEED_GAIN: 16,      // degrees added at top speed
  FOV_NITRO_GAIN: 9,       // extra degrees while boosting
  FOV_LAG:        3.2,     // FOV easing (/s)
};

/** Smoothstep helper (allocation-free). */
function crSmooth(t) { return t * t * (3 - 2 * t); }

/** Deterministic hash -> 0..1. Stable per index across frames, so per-
 *  segment surface variation never shimmers while driving. */
function crHash(n) {
  const s = Math.sin(n * 127.1 + 311.7) * 43758.5453;
  return s - Math.floor(s);
}

/** Build a closed-circuit track: straights, curves and hills in a fixed
 *  order so every run races the same circuit (skill, not luck).
 *
 *  Curvature and hill-slope EASE between neighbouring sections, so the
 *  road bends and rolls continuously instead of kinking at every section
 *  boundary — this is what makes corners read as smooth arcs rather than
 *  disconnected straight-ish chunks. */
function crBuildTrack() {
  /* P4.1 · effective lap length raised so a normal Medium lap takes
   * ~40–60s. The circuit keeps the same character (same corner order,
   * same hill profile, closed loop) but every section is ~5× longer:
   * 450 → 2250 segments = 90k → 450k world units per lap. At the
   * Medium cruise (~9–11k u/s with corners) that is a 45–55s lap.
   * Checkpoint count is unchanged (4 gates/lap incl. finish); they are
   * spaced evenly by the existing crMakeCheckpoints(total, 4). */
  const K = 5;                        // length multiplier per section
  const secs = [
    { n: 50*K, curve: 0,  hill: 0 },
    { n: 40*K, curve: 2,  hill: 10 },
    { n: 30*K, curve: 0,  hill: -6 },
    { n: 40*K, curve: -3, hill: 0 },
    { n: 30*K, curve: 0,  hill: 8 },
    { n: 35*K, curve: 3,  hill: -8 },
    { n: 25*K, curve: 0,  hill: 0 },
    { n: 45*K, curve: -2, hill: 6 },
    { n: 30*K, curve: 4,  hill: 0 },
    { n: 40*K, curve: 0,  hill: -10 },
    { n: 35*K, curve: -4, hill: 0 },
    { n: 50*K, curve: 0,  hill: 0 },
  ];
  const total = secs.reduce((a, x) => a + x.n, 0);
  const starts = [];
  let acc = 0;
  for (const x of secs) { starts.push(acc); acc += x.n; }

  // --- per-segment curvature + hill slope, eased across boundaries -----
  const EZ = 0.34;                    // fraction of a section used to blend
  const curves = new Float32Array(total);
  const slopes = new Float32Array(total);
  for (let si = 0; si < secs.length; si++) {
    const sec = secs[si], st = starts[si];
    const pv = secs[(si + secs.length - 1) % secs.length];
    const nx = secs[(si + 1) % secs.length];
    for (let i = 0; i < sec.n; i++) {
      const t = sec.n > 1 ? i / sec.n : 1;
      let cv = sec.curve, hl = sec.hill;
      if (t < EZ) {
        const k = crSmooth(t / EZ);
        cv = pv.curve + (sec.curve - pv.curve) * k;
        hl = pv.hill + (sec.hill - pv.hill) * k;
      } else if (t > 1 - EZ) {
        const k = crSmooth((t - (1 - EZ)) / EZ);
        cv = sec.curve + (nx.curve - sec.curve) * k;
        hl = sec.hill + (nx.hill - sec.hill) * k;
      }
      curves[st + i] = cv;
      slopes[st + i] = hl;
    }
  }

  // --- integrate height, then close the loop on the final straight -----
  const segs = new Array(total);
  let y = 0;
  for (let i = 0; i < total; i++) {
    // Surface tint is fixed per segment: subtle asphalt variation that is
    // stable frame to frame (no shimmer) and free to draw (precomputed).
    segs[i] = { curve: curves[i], y, sprites: [],
                tint: Math.floor(crHash(i) * 4),
                seam: (i % 9) === 0 };
    y += slopes[i];
  }
  const residual = y;                        // must return to 0 to close
  const fixStart = starts[starts.length - 1];
  const fixLen = Math.max(1, total - fixStart);
  for (let i = fixStart; i < total; i++) {
    segs[i].y -= residual * crSmooth((i - fixStart + 1) / fixLen);
  }

  /* Roadside scenery, varied so the world does not repeat obviously:
   *  • street lamps every 7 segments, alternating verges — their regular
   *    rhythm is the strongest cue for judging speed and road width
   *  • reflective pylons (delineators) on the verges
   *  • neon billboards on long straights (landmarks + sense of place)
   *  • dark conifers scattered on the far verge
   *  • glowing chevron arrows on the OUTSIDE of sharp bends, placed a few
   *    segments BEFORE the turn so the player can read it early
   */
  for (let i = 0; i < total; i++) {
    const seg = segs[i];
    if (i % 7 === 0) seg.sprites.push({ kind: 'lamp', side: (i / 7) % 2 ? 1 : -1 });
    if (i % 12 === 5) seg.sprites.push({ kind: 'pylon', side: (i / 12) % 2 ? -1 : 1 });
    if (i % 17 === 9) seg.sprites.push({ kind: 'tree', side: -1 });
    if (i % 23 === 11) seg.sprites.push({ kind: 'billboard', side: 1, hue: (i * 37) % 360 });
    if (i % 23 === 4 && i > 20) seg.sprites.push({ kind: 'billboard', side: -1, hue: (i * 53) % 360 });
    /* PHASE 5 · overhead gantry every ~45 segments: full-width sign
     * bridges are the strongest depth landmark on a real motorway — they
     * whoosh past overhead and make the corridor feel enclosed. */
    if (i % 45 === 30) seg.sprites.push({ kind: 'gantry', hue: (i * 29) % 360 });
  }
  for (let si = 0; si < secs.length; si++) {
    const sec = secs[si];
    if (Math.abs(sec.curve) >= 2) {
      const st = starts[si];
      // Arrows sit on the outside of the bend (opposite the turn), leading in.
      const side = sec.curve > 0 ? -1 : 1;
      for (let k = 0; k < 5; k++) {
        const i = (st - 12 + k * 3 + total) % total;
        segs[i].sprites.push({ kind: 'arrow', side, dir: Math.sign(sec.curve) });
      }
    }
  }
  return segs;
}

/* ---- 3. Game state ---------------------------------------------------- */
const CRG = {
  state: 'ready',            // ready | countdown | playing | paused | over
  mode: 'endurance',         // endurance | timeattack
  segs: null,                // track segments
  totalSegs: 0,
  position: 0,               // camera z along the track (world units)
  playerX: 0,                // player lateral offset, -1..1 (road edges)
  speed: 0,                  // world units / second
  maxSpeed: 0,
  nitroSpeed: 0,
  laps: 0,
  lapStart: 0,
  bestLapMs: 0,
  raceTimeMs: 0,
  distance: 0,               // total travelled (for score/odometer)
  score: 0,
  passed: 0,                 // traffic cars overtaken
  crashes: 0,
  nitro: 1,                  // 0..1 charge
  nitroActive: false,
  traffic: [],               // AI cars
  overlay: 'ready',
  countdown: 0,              // 3..2..1..GO
  timeLeftMs: 0,             // time attack only
  targetScore: 0,            // endurance goal
  shakeT: 0,                 // screen-shake timer (seconds)
  flashT: 0,                 // nitro/impact flash
  crashGrace: 0,             // post-impact invulnerability (seconds)
  steer: 0,                  // smoothed steering input (-1..1)
  roll: 0,                   // visual body roll for the player sprite
  camX: 0,                   // smoothed chase-camera lateral position
  camY: 0,                   // smoothed camera height (follows hills)
  vlat: 0,                   // lateral velocity -> momentum + drift
  drifting: false,           // grip state this frame
  driftT: 0,                 // how long the current drift has lasted
  driftScore: 0,             // points earned from the current drift
  wheelSpin: 0,              // accumulated wheel rotation (radians)
  braking: false,            // brake lights + weight transfer
  nearMiss: 0,               // near-miss flash timer
  nearMissK: null,           // last near-miss text for the HUD popup
  nearMisses: 0,             // PHASE 4: total near misses (telemetry)
  maxSpeedHit: 0,            // PHASE 4: fastest speed reached this run
  nitroUsedS: 0,             // PHASE 4: seconds of nitro burned this run
  bestDrift: 0,              // PHASE 4: biggest single banked drift
  combo: 0,                  // PHASE 4: consecutive drift chain multiplier
  comboT: 0,                 // PHASE 4: time left to extend the chain
  // PHASE 4: arcade transmission model (deliberately NOT a real gearbox —
  // see crGearModel below). rpm 0..1 eased for the tacho needle.
  rpm: 0, gear: 1,
  popups: [],                // transient score popups {text,t,x,y}
  fov: CR.FOV,               // current effective field of view (degrees)
  nitroVis: 0,               // eased 0..1 — ALL nitro visuals read this
  smoke: [],                 // tyre-smoke particles (reused pool)
  skids: [],                 // recent skid marks on the road
  offRoad: false,
  result: null,              // final summary for the overlay
  cleanup: [],
  epoch: 0,
  myEpoch: 0,
  raf: 0,
  lastT: 0,
  keys: Object.create(null),
  touch: { left: false, right: false, gas: false, brake: false, nitro: false, drift: false },
  reducedMotion: false,
  highScore: 0,
};

/* Resolve the ACTIVE car spec (multipliers on CRP base values). */
function crCarSpec() {
  const want = (typeof CRG !== 'undefined' && CRG.carId) || CR_SAVE.car;
  const car = CR_CARS.find(c => c.id === want) || CR_CARS[0];
  return { car, P: {
    // Per-run physics table: CRP scaled by the car's multipliers.
    MAX_SPEED:      CRP.MAX_SPEED * car.top,
    ACCELERATION:   CRP.ACCELERATION * car.accel,
    NITRO_ACCEL:    CRP.NITRO_ACCEL * car.accel,
    STEERING_RESPONSE: CRP.STEERING_RESPONSE * car.steer,
    STEER_AUTH_BASE:   CRP.STEER_AUTH_BASE * car.steer,
    LATERAL_FRICTION:  CRP.LATERAL_FRICTION * car.grip,
    DRIFT_GRIP:        CRP.DRIFT_GRIP * car.drift,
    NITRO_CAPACITY:    CRP.NITRO_CAPACITY * car.nitroK,
  } };
}

/** Fresh run state. Phase 6: mode + difficulty + selected car shape the
 *  run (traffic count, opponent speeds, end conditions, physics scale). */
function crNewGame(mode, diff) {
  const keep = CRG.highScore;
  const segs = crBuildTrack();
  /* BUGFIX 10 · the minimap outline derives from the track geometry, so
   * it must be rebuilt whenever a fresh track is created. */
  CR_MINIMAP.pts = null;
  mode = mode || 'quick';
  diff = diff || 'medium';
  const M = CR_MODES[mode] || CR_MODES.quick;
  const D = CR_DIFFS[diff] || CR_DIFFS.medium;
  const spec = crCarSpec();
  const maxSpeed = spec.P.MAX_SPEED * (mode === 'timeattack' ? 1.05 : 1);
  /* TASK 7.1 · fresh lap-race state for every new game object. Runs
   * BEFORE the return so every caller (restart, launch, menu) gets it. */
  CR_LAP.enabled = mode === 'quick';
  CR_LAP.totalLaps = (M.laps || 3);
  CR_LAP.lap = 1;
  CR_LAP.nextCp = 1;                 // gate 0 (finish) is behind us at start
  CR_LAP.cpFlash = 0; CR_LAP.cpFlashText = '';
  CR_LAP.lapFlash = 0; CR_LAP.lapFlashText = '';
  CR_LAP.lapTimes = [];
  CR_LAP.prevLapMs = 0; CR_LAP.bestLapMs = 0;
  CR_LAP.finished = false;
  /* TASK 7.2 · register the race field (player + AI opponents since
   * BUGFIX 3). Non-lap modes get an empty field — never a fabricated grid. */
  crRacersReset(mode, diff);
  return {
    state: 'ready', mode, diff,
    segs, totalSegs: segs.length,
    position: 0, playerX: 0, speed: 0,
    maxSpeed, nitroSpeed: maxSpeed * CRP.NITRO_MULTIPLIER,
    laps: 0, lapStart: 0, bestLapMs: 0,
    raceTimeMs: 0, distance: 0, score: 0, passed: 0, crashes: 0,
    nitro: 1, nitroActive: false,
    traffic: [], overlay: 'ready', countdown: 0,
    timeLeftMs: mode === 'timeattack' ? M.startMs : 0,
    /* TASK 7.1 · Quick Race & Championship events are lap races now.
     * Endless stays endless (no finish); Time Attack keeps its clock.
     * cps is the ordered gate list on the real track geometry. */
    cps: (mode === 'quick') ? crMakeCheckpoints(segs.length, CR_CHECKPOINTS) : null,
    targetScore: mode === 'quick' ? 0 : 0,
    quickLaps: M.laps || 0,
    shakeT: 0, flashT: 0, crashGrace: 0, steer: 0, roll: 0, camX: 0, camY: 0,
    camBob: 0, _bobT: 0, _dustAcc: 0, _sparkAcc: 0, slowmo: 0,
    vlat: 0, drifting: false, driftT: 0, driftScore: 0, wheelSpin: 0,
    driftTotal: 0, champRun: false,
    braking: false, nearMiss: 0, nearMissK: null, nearMisses: 0,
    maxSpeedHit: 0, nitroUsedS: 0, bestDrift: 0, combo: 0, comboT: 0,
    rpm: 0, gear: 1, popups: [],
    fov: CR.FOV, nitroVis: 0, smoke: [], skids: [], offRoad: false,
    result: null, highScore: keep,
  };
}

/* ---- 4. Traffic ------------------------------------------------------- */
const CR_CAR_COLORS = ['#ff5ca8', '#6fb7ff', '#b48cff', '#ffc857', '#5ef2c0', '#ff6b81'];
function crSpawnTraffic(count) {
  /* PHASE 5 · stratified + occupancy-checked spawn. Lanes are filled
   * round-robin so no lane starts empty, and every placement is probed
   * against the cars already placed (same lane AND neighbours) so no two
   * cars ever begin stacked — the failure mode of the old uniform-random
   * placement, which could put three cars in one slot.
   *
   * P3 rules baked in:
   *   • initial placement spreads over the WHOLE loop (not just ahead),
   *     so nothing materialises near the player at race start;
   *   • at most LANES-1 cars per longitudinal "spawn group" — one lane
   *     always stays free for the player to use;
   *   • every car starts with a 0.4s fade/scale-in window (spawnT) so
   *     even a recycled car eases into view instead of popping. */
  const cars = [];
  const laneW = 2 / CR.LANES;
  const gap = Math.max(20, CRG.totalSegs / count);   // target spacing (segs)
  let zCursor = 30;
  for (let i = 0; i < count; i++) {
    const lane = i % CR.LANES;                        // round-robin lanes
    const xT = -1 + laneW * (lane + 0.5);
    let placed = false;
    for (let attempt = 0; attempt < 10 && !placed; attempt++) {
      const zT = (zCursor + attempt * gap * 0.7 + Math.random() * gap * 0.5)
                 % (CRG.totalSegs - 8);
      /* P3.5 · one lane free per spawn group: reject a slot if two other
       * cars already sit within the same ~15-segment group regardless of
       * lane — that would occupy all three lanes at once. */
      let groupCount = 0;
      let clear = true;
      for (const o of cars) {
        let rz = o.z - zT;
        if (rz < -CRG.totalSegs / 2) rz += CRG.totalSegs;
        if (rz > CRG.totalSegs / 2) rz -= CRG.totalSegs;
        if (Math.abs(rz) < 15) groupCount++;
        if (Math.abs(rz) < 15 && Math.abs(o.x - xT) < laneW * 1.6) {
          clear = false; break;
        }
      }
      if (clear && groupCount >= CR.LANES - 1) clear = false;
      if (clear) {
        const car = crMakeCar();
        car.z = zT; car.x = xT; car.lane = lane; car.targetLane = lane;
        car.spawnT = 0.4;                    // P3.2 · fade/scale-in window
        cars.push(car);
        zCursor = zT + gap * 0.55;
        placed = true;
      }
    }
    if (!placed) {                    // fall back: skip this slot entirely
      /* BUGFIX 7 · the old fallback accepted zCursor blindly, which could
       * stack a car onto an existing one. Skipping keeps the field one
       * car lighter for a moment — invisible next to a clean road. */
      zCursor += gap * 0.8;
    }
  }
  return cars;
}
function crMakeCar(minAhead) {
  const lane = Math.floor(Math.random() * CR.LANES);
  // Lateral centre of each lane in playerX units (-1..1 road space).
  const laneW = 2 / CR.LANES;
  const x = -1 + laneW * (lane + 0.5);
  const ahead = minAhead !== undefined
    ? minAhead
    : 25 + Math.random() * (CRG.totalSegs - 40);   // segments ahead
  /* PHASE 5 · five body styles. Widths are road-units; heights are the
   * drawn height/width ratio used by the renderer. The mix is weighted so
   * the road reads as ordinary traffic with occasional interest.
   *   hatch  compact runabout    (narrow, tall cabin)
   *   sedan  classic three-box   (the baseline)
   *   sport  low wedge + wing    (fast, rare)
   *   van    tall boxy hauler    (big obstacle, slow)
   *   truck  long high-sided rig (landmark obstacle, slowest) */
  const r = Math.random();
  const style = r < 0.30 ? 'hatch'
              : r < 0.62 ? 'sedan'
              : r < 0.78 ? 'sport'
              : r < 0.92 ? 'van'
                         : 'truck';
  const dims = style === 'truck' ? { w: 0.80, h: 1.00 }
             : style === 'van'   ? { w: 0.74, h: 0.86 }
             : style === 'sport' ? { w: 0.56, h: 0.50 }
             : style === 'hatch' ? { w: 0.54, h: 0.68 }
                                 : { w: 0.62, h: 0.64 };
  // Trucks/vans cruise slower; sports cars hustle.
  const spdK = style === 'truck' ? 0.72
             : style === 'van'   ? 0.80
             : style === 'sport' ? 1.22
             : style === 'hatch' ? 0.95 : 1;
  /* Driver personality — every AI car gets its own temperament so traffic
   * reads as individuals, not a swarm:
   *   aggression  how willingly it changes lanes / overtakes
   *   speedBias   steady per-car cruise offset (some dawdle, some hustle)
   *   wander      lane-centre noise amplitude
   *   reaction    how fast it responds to a blocked lane (seconds) */
  const aggression = 0.15 + Math.random() * 0.85;
  /* P3.7 · ordinary traffic cruises at 40–70% of the player's rated
   * speed (style multiplier aside), with a hard floor so nobody crawls.
   * The old band (26–56%) let vans/trucks drop to ~19% — near-stationary
   * walls rather than flowing traffic. */
  const baseK = Math.max(0.40, Math.min(0.70, 0.40 + Math.random() * 0.30));
  return {
    z: (CRG.position / CR.SEG_LEN + ahead) % CRG.totalSegs,  // segment index
    x,
    lane,                                          // current lane index
    targetLane: lane,                              // lane it is moving toward
    speed: baseK * CRG.maxSpeed * spdK,
    color: CR_CAR_COLORS[Math.floor(Math.random() * CR_CAR_COLORS.length)],
    w: dims.w, h: dims.h, style,
    passedFlag: false,
    nearMissed: false,
    drift: (Math.random() - 0.5) * 0.14,           // slow lane wander
    aggression,
    speedBias: (Math.random() - 0.5) * 0.06,       // ±3% cruise variation
    wander: 0.02 + Math.random() * 0.05,
    laneT: 0,                                      // time since last decision
    reaction: 0.35 + Math.random() * 0.5,
    bumped: 0,                                     // collision reaction timer
    spawnT: 0.4,                                   // P3.2 · fade/scale-in (s)
  };
}

/* ---- 5. Physics step -------------------------------------------------- */
/** Transient HUD popup (drift payout, near miss...). Self-expiring; drawn
 *  on the canvas above the car so it never fights the DOM HUD. */
function crPopup(text, color) {
  const g = CRG;
  /* BUGFIX 9 · repeated NEAR MISS events combine into ONE counting
   * popup ("NEAR MISS ×3") instead of spawning overlapping copies: if
   * the newest popup is the same event type and still fresh (<0.5s
   * old), bump its count and refresh its timer. Other event types keep
   * their own popups. */
  const isNearMiss = text.startsWith('NEAR MISS');
  if (isNearMiss && g.popups.length) {
    const last = g.popups[g.popups.length - 1];
    if (last.nearMissCount && last.t > 0.65) {
      last.nearMissCount++;
      last.text = `NEAR MISS +${120 * last.nearMissCount}`;
      last.t = 1.15;                          // refresh lifetime
      return;
    }
  }
  if (g.popups.length > 4) g.popups.shift();  // hard cap
  g.popups.push({ text, color: color || '#e8ecf4', t: 1.15,
                  x: (Math.random() - 0.5) * 40,
                  nearMissCount: isNearMiss ? 1 : 0 });
}

/** Emit tyre smoke while drifting. Particles live in a fixed pool that is
 *  reused frame to frame — no allocation churn at 60fps.
 *  Phase 3: density now scales with drift INTENSITY (slip angle × speed),
 *  and the plume is emitted from the OUTSIDE wheel of the slide, drifting
 *  directionally with the car's lateral motion — so a hard drift leaves a
 *  visibly thicker, sideways-swept curtain than a gentle one. */
function crEmitSmoke(dt) {
  const g = CRG;
  const P = CRP;
  // Intensity 0..1: slip beyond the drift threshold, saturating at 2× it.
  const slip = Math.abs(g.vlat);
  const inten = Math.min(1, Math.max(0,
    (slip - P.DRIFT_SLIP_MIN) / P.DRIFT_SLIP_MIN));
  const rate = 34 + 46 * inten;                    // 34..80 particles/s
  g.smokeAcc = (g.smokeAcc || 0) + rate * dt;
  // Directional bias: smoke sweeps the way the car is sliding.
  const dirX = g.vlat > 0 ? 1 : -1;
  while (g.smokeAcc >= 1) {
    g.smokeAcc -= 1;
    if (g.smoke.length < 90) {
      g.smoke.push({
        // Smoke originates at the rear wheels, in road-space so it stays
        // glued to the world as the car moves away from it.
        z: g.position - 120,
        x: g.playerX + (Math.random() - 0.5) * 0.16,
        life: 1, r: 26 + Math.random() * 22 + inten * 14,
        vy: -(14 + Math.random() * 22),
        // Phase 3: lateral drift of the puff, biased into the slide.
        vx: dirX * (10 + Math.random() * 26) + (Math.random() - 0.5) * 14,
      });
    }
  }
}

/** Advance + cull effect particles. Called once per physics tick. */
function crEffectsStep(dt) {
  const g = CRG;
  for (let i = g.smoke.length - 1; i >= 0; i--) {
    const p = g.smoke[i];
    p.life -= dt * 1.35;
    p.r += dt * 46;
    if (p.life <= 0) g.smoke.splice(i, 1);
  }
  for (let i = g.popups.length - 1; i >= 0; i--) {
    g.popups[i].t -= dt;
    if (g.popups[i].t <= 0) g.popups.splice(i, 1);
  }
  if (g.nearMiss > 0) g.nearMiss = Math.max(0, g.nearMiss - dt);
  /* TASK 7.1 · banner timers decay with the simulation clock so a paused
   * race freezes them exactly like every other effect. */
  if (CR_LAP.cpFlash > 0) CR_LAP.cpFlash = Math.max(0, CR_LAP.cpFlash - dt);
  if (CR_LAP.lapFlash > 0) CR_LAP.lapFlash = Math.max(0, CR_LAP.lapFlash - dt);
  // Skid marks fade over ~2.5s.
  for (let i = g.skids.length - 1; i >= 0; i--) {
    g.skids[i].t -= dt;
    if (g.skids[i].t <= 0) g.skids.splice(i, 1);
  }
}

/* =====================================================================
   PHASE 4 · ARCADE TRANSMISSION MODEL
   This is deliberately NOT a simulated gearbox: the physics engine has one
   continuous speed curve, so a real ratio-based transmission would be a
   lie. Instead this is an explicit arcade model:
     • Six virtual gears with speed windows derived from the BASE cap
       (g.maxSpeed), so gear N always means the same fraction of the car's
       rated ability — engaging nitro must never read as a downshift.
     • RPM is the position inside the current gear's window, blended with
       the throttle so lifting off visibly drops revs.
     • On an upshift the needle dips for ~180ms (the "shift kick") — pure
       presentation, zero influence on the physics.
     • Nitro overspeed (above maxSpeed) simply over-revs 6th gear: the
       needle pins near redline, which is the classic arcade boost read.
   ===================================================================== */
const CR_GEARS = 6;                       // arcade gear count
function crGearModel(g, dt, gas) {
  // Windows are anchored to the BASE cap: nitro extends speed beyond the
  // top of 6th, which clamps to gear 6 + over-rev instead of "shifting".
  const sr = Math.min(1, g.speed / g.maxSpeed);     // 0..1 of rated cap
  // Gear = which sixth of the cap we are in (1-indexed, clamped).
  const gear = Math.max(1, Math.min(CR_GEARS,
    Math.floor(sr * CR_GEARS) + 1));
  // Position inside this gear's window (0..1).
  const lo = (gear - 1) / CR_GEARS, hi = gear / CR_GEARS;
  let inGear = (sr - lo) / (hi - lo);
  inGear = Math.max(0, Math.min(1, inGear));
  // Throttle blend: closed throttle pulls revs down ~35%.
  let target = gas ? 0.25 + inGear * 0.75 : inGear * 0.62;
  // Nitro over-rev flare: pinned near redline while boosting.
  if (g.nitroActive) target = Math.max(target, 0.92);
  // Shift dip: right after an upshift the needle falls briefly.
  if (gear > g.gear) { g._shiftT = 0.18; crSfxGear(); }   // PHASE 6 blip
  if (g._shiftT > 0) { g._shiftT -= dt; target *= 0.78; }
  // Smooth needle (never snaps, even on hard hits).
  g.rpm += (target - g.rpm) * Math.min(1, dt * 7);
  g.rpm = Math.max(0, Math.min(1, g.rpm));
  g.gear = gear;
}

/** Record a skid-mark pair at the rear wheels (called when a drift starts
 *  and periodically while it lasts). Kept short so the array stays tiny. */
function crAddSkid() {
  const g = CRG;
  if (g.skids.length > 24) g.skids.shift();
  g.skids.push({ z: g.position, x: g.playerX, t: 2.5 });
}

/** One simulation tick. dt is clamped by the caller. */
function crStep(dt) {
  const g = CRG;
  const P = CRP;                       // physics config (single source)
  /* PHASE 6 · vehicle character: overlay the selected car's multipliers
   *  onto the base table. Only the seven stats that define a car's feel
   *  are scaled — collision, camera and drift-scoring stay identical so
   *  fairness and the Phase 2 contract are untouched. */
  const carP = crCarSpec().P;
  const MAX_SPEED    = carP.MAX_SPEED;
  const ACCELERATION = carP.ACCELERATION;
  const NITRO_ACCEL  = carP.NITRO_ACCEL;
  const STEER_RESP   = carP.STEERING_RESPONSE;
  const STEER_AUTH   = carP.STEER_AUTH_BASE;
  const LAT_FRICTION = carP.LATERAL_FRICTION;
  const DRIFT_GRIP   = carP.DRIFT_GRIP;
  const NITRO_CAP    = carP.NITRO_CAPACITY;
  const K = g.keys;
  const T = g.touch;

  const gas = K['ArrowUp'] || K['w'] || K['W'] || T.gas;
  const brake = K['ArrowDown'] || K['s'] || K['S'] || T.brake;
  const left = K['ArrowLeft'] || K['a'] || K['A'] || T.left;
  const right = K['ArrowRight'] || K['d'] || K['D'] || T.right;
  /* Nitro: Space (or N, or the touch pad). Shift is now DRIFT. */
  const wantNitro = K[' '] || K['n'] || K['N'] || T.nitro;
  const wantDrift = K['Shift'] || T.drift;

  const speedRatio = g.speed / g.maxSpeed;

  // ================= NITRO =================
  // Engages only with a non-empty meter AND enough speed to matter; the
  // boost must never bypass collision or road-boundary logic, so it only
  // raises the engine cap and pull — all drag/grass rules still apply.
  const canNitro = g.nitro > P.NITRO_MIN_ENERGY && wantNitro && gas
                && g.speed > g.maxSpeed * P.NITRO_MIN_SPEED;
  if (canNitro && !g.nitroActive) crSfx('nitro');
  g.nitroActive = canNitro;
  if (g.nitroActive) {
    // Drain: dt / capacity, where capacity is the selected car's scaled
    // value (NITRO_CAP = CRP.NITRO_CAPACITY * car.nitroK — see crCarSpec).
    g.nitro = Math.max(0, g.nitro - dt / NITRO_CAP);   // dt / P.NITRO_CAPACITY base
  } else {
    // Recharges while driving fast (rewarding clean racing).
    const regen = g.speed > g.maxSpeed * 0.6 ? P.NITRO_REGEN_FAST : P.NITRO_REGEN_SLOW;
    g.nitro = Math.min(1, g.nitro + dt * regen);
  }
  // Phase 3: every nitro visual eases through this one scalar so boost
  // engages and releases as a smooth swell, never a hard cut.
  g.nitroVis += ((g.nitroActive ? 1 : 0) - g.nitroVis) * Math.min(1, dt * 5.5);

  // ================= LONGITUDINAL =================
  const top = g.nitroActive ? g.nitroSpeed : g.maxSpeed;
  const sr = g.speed / top;                       // 0..1 toward current cap
  if (gas) {
    const baseAccel = g.nitroActive ? NITRO_ACCEL : ACCELERATION;
    // Torque curve: full pull at 0 revs, tapering toward the cap.
    const torque = 1 - P.TORQUE_FALLOFF * sr * sr;
    g.speed = Math.min(top, g.speed + baseAccel * torque * dt);
  } else if (brake) {
    g.speed = Math.max(0, g.speed - P.BRAKE_FORCE * dt);
  } else {
    g.speed = Math.max(0, g.speed - P.ENGINE_BRAKE * dt);   // coasting
  }
  // Losing nitro mid-burn must bleed the excess smoothly, never snap.
  if (g.speed > top) g.speed = Math.max(top, g.speed - P.OVERSPILL_BLEED * dt);

  // ================= STEERING (inertia + speed sensitivity) =================
  // Authority rises quickly from standstill then eases at high speed; the
  // input ramps over ~130ms so taps feel progressive, not binary. Release
  // lets `steer` ease back to 0 by the same ramp — smooth self-centring.
  const steerAuth = STEER_AUTH
    * (P.STEER_AUTH_LO + (1 - P.STEER_AUTH_LO) * Math.min(1, speedRatio * P.STEER_AUTH_RAMP));
  const steerInput = (right ? 1 : 0) - (left ? 1 : 0);
  g.steer = g.steer === undefined ? 0 : g.steer;
  const delta = Math.max(-STEER_RESP * dt,
                  Math.min(STEER_RESP * dt, steerInput - g.steer));
  g.steer += delta;

  // ================= LATERAL DYNAMICS: momentum, grip, DRIFT =================
  const vRel2 = g.speed / g.maxSpeed;              // 0..1
  const off1 = Math.abs(g.playerX) - P.ROAD_HALF;  // >0 when off tarmac
  const onGrass = off1 > 0;

  /* ACTIVE DRIFT: holding Shift unloads the rear tyres. Grip drops, so the
   * car slides further for the same steering input — a real technique, not
   * an auto-slide. The slide needs speed AND meaningful steering to begin,
   * and continues only while some steering is held (counter-steer keeps it
   * alive), otherwise grip gradually restores. */
  const driftHeld = wantDrift && vRel2 > P.DRIFT_SPEED_MIN
                 && Math.abs(g.steer) > P.DRIFT_HOLD_STEER;
  let grip = LAT_FRICTION - P.GRIP_SPEED_LOSS * vRel2 * vRel2;
  if (driftHeld) grip *= DRIFT_GRIP;
  else if (g.drifting) grip *= P.DRIFT_GRIP_RECOVER;   // sliding after release
  if (onGrass) grip *= P.GRASS_GRIP;
  /* NATURAL STEERING FIX · grip recovers sharply once the player releases
   * the wheel: with no steering input and no active drift the tyres bite
   * again at ~1.6× base grip, so lateral velocity decays in ~0.15–0.25s
   * instead of the old ~0.6s+ tail-off. The car now STOPS where the input
   * stops (crisp, planted feel) while genuine drifts keep their long,
   * controlled slides. Only applies to the no-input coast case, so drift
   * recovery and grass behaviour are untouched. */
  if (!driftHeld && !g.drifting && Math.abs(steerInput) < 0.05)
    grip *= P.GRIP_RELEASE_BITE;

  // Sideways acceleration = steering force; grip is EXPONENTIAL DECAY
  // (frame-rate independent) so velocity settles at a real equilibrium
  // (vlat ~= steerForce/grip) instead of being pinned at zero.
  const steerForce = g.steer * steerAuth
    * (P.STEER_SPEED_MIX + (1 - P.STEER_SPEED_MIX) * vRel2);
  g.vlat += steerForce * dt;
  const gripDecay = Math.exp(-grip * dt);          // 0..1, dt-safe
  g.vlat *= gripDecay;
  // Snap tiny residuals so the car truly settles (no micro-drift).
  if (Math.abs(g.vlat) < P.VLAT_SNAP && Math.abs(g.steer) < 0.05) g.vlat = 0;

  // Drift state: sustained slip above threshold (held OR carried over).
  const slip = Math.abs(g.vlat);
  const wasDrifting = g.drifting;
  g.drifting = slip > P.DRIFT_SLIP_MIN && vRel2 > P.DRIFT_SPEED_MIN;
  if (g.drifting) {
    g.driftT += dt;
    /* Drift score rewards SKILL: duration × angle (slip) × speed, so a long
     * fast sideways slide pays far more than a slow crawl. Stationary or
     * near-stationary "drifting" is impossible — the speed gate below
     * zeroes accrual under 30% of max speed. */
    if (vRel2 > P.DRIFT_SPEED_MIN) {
      g.driftScore += dt * (60 + slip * 90) * vRel2;
    }
  } else if (wasDrifting && g.driftT > P.DRIFT_MIN_TIME) {
    // Bank the drift: long controlled slides pay out.
    const banked = Math.round(g.driftScore);
    if (banked >= P.DRIFT_BANK_MIN) {
      /* PHASE 4 · combo chain: consecutive banked drifts within a short
       * window stack a multiplier on the PAYOUT (the underlying accrual
       * is untouched). Breaking the chain (crash, timeout) resets it. */
      if (g.comboT > 0) g.combo = Math.min(9, g.combo + 1);
      else g.combo = 1;
      g.comboT = 4.5;                      // seconds to keep the chain alive
      const payout = banked * g.combo;
      g.score += payout;
      g.driftTotal += payout;
      if (payout > g.bestDrift) g.bestDrift = payout;
      // PHASE 6 · Time Attack: drift skill buys time too.
      if (g.mode === 'timeattack') g.timeLeftMs += Math.min(2500, payout * 2);
      crPopup(g.combo > 1
        ? `DRIFT +${banked} ×${g.combo} = ${payout}`
        : `DRIFT +${payout}`, '#b48cff');
      crSfx('drift');
    }
    g.driftT = 0; g.driftScore = 0;
  } else if (!g.drifting) {
    g.driftT = 0; g.driftScore = 0;
  }

  // Apply lateral movement.
  g.playerX += g.vlat * dt;

  // Car body roll follows the SLIP (not just input) so the visual matches
  // the physics: rolling into a slide reads as weight transfer. A held
  // drift adds extra counter-rotation for that "sideways" read.
  const rollTarget = -g.vlat * 0.10 - g.steer * 0.03
                   - (driftHeld ? g.vlat * 0.06 : 0);
  g.roll = g.roll === undefined ? 0 : g.roll;
  g.roll += (rollTarget - g.roll) * Math.min(1, dt * 8);

  // Wheel spin drives tyre rotation + smoke emission rate.
  g.wheelSpin += (g.speed / CR.WHEEL_R) * dt;
  if (g.drifting && !g.reducedMotion) crEmitSmoke(dt);

  // Curve pull: centrifugal push toward the OUTSIDE of a bend, scaled by
  // speed squared — what makes fast corners demand real commitment.
  const segIndex = Math.floor(g.position / CR.SEG_LEN) % g.totalSegs;
  const seg = g.segs[segIndex];
  g.playerX -= seg.curve * P.CURVE_PULL * vRel2 * vRel2 * dt;

  // ================= ROAD BOUNDARIES =================
  // Heavy drag plus a self-centring pull, so the car cannot be parked on
  // the verge: it scrubs speed and slides back unless steered out.
  const off = Math.abs(g.playerX) - P.ROAD_HALF;
  if (off > 0) {
    g.speed = Math.max(g.maxSpeed * P.OFFROAD_MIN_FR,
                       g.speed - (P.OFFROAD_DRAG + off * P.OFFROAD_DRAG_K) * dt);
    // Self-centring with a smooth falloff near the boundary so the car
    // settles ON the tarmac edge instead of oscillating across it.
    const edgeEase = Math.min(1, off / P.EDGE_EASE);
    g.playerX -= Math.sign(g.playerX)
               * (P.SELF_CENTRE + off * P.SELF_CENTRE_K) * edgeEase * dt;
    if (!g.offRoad) crSfx('rumble');
    // Grass kills lateral velocity quickly (digging in).
    if (Math.abs(g.vlat) > 0.01) g.vlat *= Math.max(0, 1 - 6 * dt);
  }
  g.offRoad = off > 0;
  // Hard barrier clamp — never teleports, just stops at the wall.
  g.playerX = Math.max(-P.BOUND_MAX, Math.min(P.BOUND_MAX, g.playerX));

  // ================= CAMERA =================
  // Chase camera eases toward the car with a fixed time-constant (reads as
  // damping, not rubber-banding). Height follows the road for crests/dips.
  // Phase 3 adds three refinements, all still allocation-free:
  //   • drift widens the follow (camera leans OUT of the slide)
  //   • a speed-scaled vertical suspension bob
  //   • post-collision stabilization: the lag tightens so the camera
  //     re-centres decisively instead of wandering after a hit.
  /* PHASE 5 · predictive chase camera. The camera now looks slightly
   * INTO the upcoming corner (sampled a few segments ahead), which does
   * two things: the apex is visible earlier (depth perception), and the
   * car appears to move within the frame on corner entry instead of the
   * whole world pivoting around a fixed car. All Phase 3 behaviour
   * (drift widen, post-hit tighten, bob) is preserved — this only adds
   * a small look-ahead term to the target. */
  const aheadSeg = g.segs[(Math.floor(g.position / CR.SEG_LEN) + 14) % g.totalSegs];
  const lookAhead = aheadSeg.curve * 0.055;        // gentle corner anticipation
  const driftK = g.drifting ? P.CAM_DRIFT_K : 1;
  const camTarget = (g.playerX * P.CAM_INSIDE + lookAhead) * driftK;
  const camLag = (g.crashGrace > 0 ? P.CAM_LAG_STAB : P.CAM_LAG) * driftK;
  g.camX += (camTarget - g.camX) * Math.min(1, dt * camLag);
  g.camY += (seg.y - g.camY) * Math.min(1, dt * P.CAM_H_LAG);

  // Suspension bob: a gentle sine driven by wheelSpin (so it is tied to
  // actual motion, not wall-clock), scaled by speed and muted at rest.
  if (!g.reducedMotion) {
    g._bobT = (g._bobT || 0) + dt * (2.2 + vRel2 * 5.5);
    g.camBob = Math.sin(g._bobT) * (0.4 + vRel2 * 2.6);
  } else {
    g.camBob = 0;
  }

  // Speed-dependent FOV widens for a sense of rush; nitro pushes further.
  // Phase 3: the nitro gain now eases through nitroVis so the FOV swell
  // and its release are both smooth.
  const fovTarget = CR.FOV + P.FOV_SPEED_GAIN * Math.min(1, vRel2 * vRel2)
                  + g.nitroVis * P.FOV_NITRO_GAIN;
  g.fov += (fovTarget - g.fov) * Math.min(1, dt * P.FOV_LAG);

  // Nitro adds a subtle continuous shake (tiny: comfort first).
  if (g.nitroActive && !g.reducedMotion) {
    g.shakeT = Math.max(g.shakeT, P.NITRO_SHAKE);
  }

  // Brake-light state for rendering (held this frame?).
  g.braking = !!brake && g.speed > 1;

  // ================= PHASE 4 · TELEMETRY + TRANSMISSION =================
  // All derived from live physics — nothing here feeds back into it.
  if (g.speed > g.maxSpeedHit) g.maxSpeedHit = g.speed;
  if (g.nitroActive) g.nitroUsedS += dt;
  crGearModel(g, dt, !!gas);

  // ================= ADVANCE =================
  const dz = g.speed * dt;
  g.position += dz;
  g.distance += dz;
  g.raceTimeMs += dt * 1000;

  /* TASK 7.1 · ordered-checkpoint lap detection. The old heuristic
   * (index wraparound + a 3s guard) could be fooled by reversing over
   * the line; now a lap requires every gate crossed forward in order.
   * Non-lap modes (endless/timeattack) keep the legacy counter purely as
   * an odometer of circuits travelled — it never ends their race. */
  const prevIndex = segIndex;
  const newIndex = ((Math.floor(g.position / CR.SEG_LEN) % g.totalSegs)
                    + g.totalSegs) % g.totalSegs;
  const cpEvent = crCheckpointStep(prevIndex, newIndex);
  /* BUGFIX 3 · AI opponents advance with the same simulation clock as
   * the player (they pause when the game pauses, freeze on finish). */
  if (CR_LAP.enabled && !CR_LAP.finished) {
    for (const r of CR_RACERS) if (!r.isPlayer) crAiStep(r, dt);
  }
  /* TASK 7.2 · gate/lap/finish transitions change progress by whole
   * units — recompute standings immediately at those moments (the
   * throttled path covers the smooth in-between). */
  if (cpEvent) crStandingsUpdate(true);
  if (cpEvent === 'finish') {
    return crFinish(true);                   // all laps complete: race won
  }
  if (!CR_LAP.enabled && newIndex < prevIndex && g.raceTimeMs > 3000) {
    g.laps++;                                // endless/TA circuit counter
  }
  /* TASK 7.2 · throttled standings refresh (no sort per render frame). */
  crStandingsUpdate(false);

  // --- score ---
  /* PHASE 6 · Endless: the score rate itself climbs with distance, so
   *  surviving longer is worth disproportionately more (survival scoring).
   *  Quick/Time Attack keep the flat Phase 2 rate. */
  const survK = g.mode === 'endless' ? 1 + Math.min(2, g.distance / 2000000) : 1;
  g.score += dz * 0.0022 * survK * (g.nitroActive ? 1.5 : 1);

  // --- traffic ---
  crTrafficStep(dt);

  /* --- PHASE 6 · mode rules ------------------------------------------
   * quick      : win by reaching the target score (distance-driven)
   * timeattack : countdown; speed/drift/near-miss EXTEND the clock
   * endless    : no victory condition — score is survival + skill        */
  if (g.mode === 'timeattack') {
    g.timeLeftMs -= dt * 1000;
    // Time extensions: +1.2s per near miss, +0.8s per 100 drift banked.
    // (Extensions are applied where those events fire, not here.)
    if (g.timeLeftMs <= 0) { g.timeLeftMs = 0; return crFinish(g.distance > 300000); }
  }
  /* TASK 7.1 · Quick Race completion moved to the checkpoint system: the
   * race ends when the final lap's finish line is crossed (handled above).
   * Score remains a live metric but is no longer an end condition. */
  // Endless: never ends by score; only the player quitting or crashing
  // out of interest ends it. Difficulty ramps inside crTrafficStep.

  // ================= EFFECTS DECAY =================
  if (g.shakeT > 0) g.shakeT = Math.max(0, g.shakeT - dt);
  if (g.flashT > 0) g.flashT = Math.max(0, g.flashT - dt);
  // PHASE 4: combo chain expires silently (multiplier resets on timeout).
  if (g.comboT > 0) { g.comboT -= dt; if (g.comboT <= 0) { g.comboT = 0; g.combo = 0; } }
  if (g.crashGrace > 0) g.crashGrace = Math.max(0, g.crashGrace - dt);
  crEffectsStep(dt);
  // Lay skid marks at a steady cadence while sliding.
  if (g.drifting) {
    g.skidAcc = (g.skidAcc || 0) + dt;
    if (g.skidAcc > 0.07) { g.skidAcc = 0; crAddSkid(); }
  }

  /* PHASE 6 · audio state: gear-aware drone + drift tyre squeal. The
   * squeal follows the actual drift flag (physics), not a timer. */
  crEngineUpdate(g.nitroActive ? 1 : speedRatio, g.rpm, g.gear);
  crDriftAudio(g.drifting && g.speed > g.maxSpeed * 0.25);
}


/** Move AI cars, handle overtakes and collisions. */
/** Move AI cars: cruise, lane changes, overtaking, collision reaction.
 *
 *  Each car is an independent driver with its own temperament (aggression,
 *  speed bias, wander). Behaviour rules, evaluated on a per-car cadence so
 *  the O(n) pass stays cheap:
 *   • maintain its own cruise speed (varied per car)
 *   • look ahead in its lane; if blocked by a slower car, pick an adjacent
 *     free lane and change lanes smoothly (never teleport)
 *   • faster/more aggressive cars overtake; dawdlers hold their lane
 *   • after being hit, they wobble and ease off briefly (reaction)
 *   • all movement uses the existing world coordinates (z segments, x road
 *     units), so projection/collision code is unchanged. */
/* BUGFIX 7 · occupancy probe: is (z, x) clear of every car in `list`?
 * Same-lane placements need a real following gap (>= MIN_SPAWN_GAP
 * segments); cross-lane keeps the anti-stack bubble. Wrapped distances,
 * so it is correct across the loop seam. */
function crTrafficSlotFree(z, x, list) {
  const total = CRG.totalSegs;
  const laneW = 2 / CR.LANES;
  for (const o of list) {
    let rz = o.z - z;
    if (rz < -total / 2) rz += total;
    if (rz > total / 2) rz -= total;
    const sameLane = Math.abs(o.x - x) < laneW * 0.75;
    if ((sameLane && Math.abs(rz) < 20)          // following gap, same lane
        || (!sameLane && Math.abs(rz) < 16       // anti-stack bubble
            && Math.abs(o.x - x) < laneW * 1.6)) return false;
  }
  return true;
}

function crTrafficStep(dt) {
  const g = CRG;
  const P = CRP;
  const playerSeg = g.position / CR.SEG_LEN;
  const laneW = 2 / CR.LANES;
  const laneCentre = l => -1 + laneW * (l + 0.5);
  /* PHASE 6 · difficulty drives the AI: opponents cruise faster and
   *  change lanes more willingly on higher settings, and Endless mode
   *  ramps both with distance travelled. All multipliers resolve once
   *  per tick — zero cost inside the car loop. */
  const D = CR_DIFFS[g.diff] || CR_DIFFS.medium;
  const MIN_GAP_SEGS = 2.5;                    // hard same-lane floor (segs)
  let oppK = D.oppSpeed, aggK = D.aggression;
  if (g.mode === 'endless') {
    const ramp = Math.min(1.6, g.distance / 4000000);      // +60% by 40 km
    oppK *= 1 + ramp; aggK *= 1 + ramp * 0.8;
    // Progressive density: inject a new car each ~1.4 km of ramp.
    /* P3.4 · Endless ramp also respects the difficulty traffic cap:
     * density grows with distance but never past the hard ceiling. */
    const wantCars = Math.min(CR_DIFFS.hard.traffic,
                              D.traffic + Math.floor(g.distance / 1400000));
    /* BUGFIX 7 · ramp spawns go through the SAME occupancy probe as
     * recycling: never stack a new car on (or beside) an existing one. */
    while (g.traffic.length < wantCars) {
      const cand = crMakeCar();
      if (crTrafficSlotFree(cand.z, cand.x, g.traffic)) g.traffic.push(cand);
      else break;                                   // crowded: retry next tick
    }
  }

  /* P3.2 · fade/scale-in window decays with the simulation clock; while
   * it is positive the renderer draws the car smaller and translucent.
   * Cleared here so a paused race freezes the fade too. */
  for (const car of g.traffic) {
    if (car.spawnT > 0) car.spawnT = Math.max(0, car.spawnT - dt);
  }

  for (const car of g.traffic) {
    /* --- car-following (PHASE 5) -------------------------------------
     * Find the nearest car ahead in roughly my lane. If the gap is under
     * ~12 segments, ease my speed toward theirs (never exceeding it) so
     * AI traffic queues politely instead of driving through each other.
     * This is what keeps spacing "realistic" over long runs — without it,
     * two cars with different cruise speeds eventually overlap. */
    let leader = null, leaderGap = 1e9;
    for (const other of g.traffic) {
      if (other === car) continue;
      if (Math.abs(other.x - car.x) > laneW * 0.75) continue;   // not my lane
      let rz = other.z - car.z;
      if (rz < -g.totalSegs / 2) rz += g.totalSegs;
      if (rz > g.totalSegs / 2) rz -= g.totalSegs;
      if (rz > 0.2 && rz < leaderGap) { leaderGap = rz; leader = other; }
    }
    /* TASK 7.0 · two-stage following: begin easing at 20 segments (smooth
     * compression) and clamp hard under 12, so same-lane queues keep a
     * visible gap at speed instead of closing to contact. */
    let followK = 1;
    if (leader && leaderGap < 20) {
      const ease = Math.min(1, (20 - leaderGap) / 8);        // 0..1
      const clampK = Math.max(0.45, leader.speed / Math.max(car.speed, 1));
      followK = 1 - ease * (1 - clampK);
    }
    /* TASK 7.0 · hard overlap guard: the eased clamp alone lets a fast
     * closer overshoot into the leader's slot between ticks (no true AI
     * brakes). If the gap is under the safety floor, freeze the follower
     * AT the floor — cars can queue tightly but never occupy the same
     * space. Checked against the leader found above; O(1) per car. */

    // --- cruise speed: personal bias × difficulty, eased toward target -
    /* BUGFIX 7 · cruise target is CAPPED near the player's rated speed.
     * The old formula compounded: want ∝ car.speed × oppK (up to ~2× on
     * endless ramp), so traffic accelerated exponentially to 2-3× the
     * player's top speed — cars closed hundreds of segments between
     * ticks, tunnelling through the separation floor and stacking at the
     * horizon. Road users never out-run the racer by that much. */
    let want = Math.min(car.speed * (1 + car.speedBias) * followK * oppK,
                        g.maxSpeed * 1.05);
    // Reaction to being bumped: brake a touch, then recover.
    if (car.bumped > 0) {
      car.bumped -= dt;
      want *= 0.72;
    }
    car.speed += (want - car.speed) * Math.min(1, dt * 1.6);

    car.z += (car.speed / CR.SEG_LEN) * dt;
    if (car.z >= g.totalSegs) car.z -= g.totalSegs;

    /* --- lane-keeping + lane changes ---------------------------------
     * PHASE 5: the decision now models BOTH hazards ahead (a slower car)
     * and the approaching PLAYER (much faster, from behind). AI cars
     * politely yield out of the player's lane when there is room — but
     * only sometimes and only if they are not already committed, so the
     * road never feels like it parts on rails. */
    car.laneT += dt;
    if (car.laneT > car.reaction) {
      car.laneT = 0;
      // Hazard scan: slower traffic ahead in my lane?
      let blockedBy = null;
      for (const other of g.traffic) {
        if (other === car) continue;
        let rz = other.z - car.z;
        if (rz < -g.totalSegs / 2) rz += g.totalSegs;
        if (rz > g.totalSegs / 2) rz -= g.totalSegs;
        if (rz > 0.5 && rz < 14 && other.speed < car.speed * 0.94
            && Math.abs(other.x - car.x) < laneW * 0.8) { blockedBy = other; break; }
      }
      // Player awareness: is the player closing fast from behind in my lane?
      let playerBehind = false;
      {
        // relPC: how far ahead of the PLAYER this car is (segments).
        let relPC = car.z - playerSeg;
        if (relPC < -g.totalSegs / 2) relPC += g.totalSegs;
        if (relPC > g.totalSegs / 2) relPC -= g.totalSegs;
        playerBehind = relPC > 1 && relPC < 10
          && Math.abs(car.x - g.playerX) < laneW * 0.7
          && g.speed > car.speed * 1.35;
      }
      const wantChange = (blockedBy && Math.random() < (0.35 + car.aggression * 0.55) * aggK)
                      || (playerBehind && Math.random() < (0.30 + car.aggression * 0.35) * aggK);
      if (wantChange) {
        const opts = [];
        if (car.targetLane > 0) opts.push(car.targetLane - 1);
        if (car.targetLane < CR.LANES - 1) opts.push(car.targetLane + 1);
        if (opts.length > 1 && Math.random() < 0.5) opts.reverse();
        for (const L of opts) {
          const lx = laneCentre(L);
          let free = true;
          for (const other of g.traffic) {
            if (other === car) continue;
            let rz = other.z - car.z;
            if (rz < -g.totalSegs / 2) rz += g.totalSegs;
            if (rz > g.totalSegs / 2) rz -= g.totalSegs;
            if (Math.abs(rz) < 7 && Math.abs(other.x - lx) < laneW * 0.78) {
              free = false; break;
            }
          }
          // Never change INTO the player's current position.
          if (free && Math.abs(lx - g.playerX) < laneW * 0.55
              && Math.abs(car.z - playerSeg) < 4) free = false;
          if (free) { car.targetLane = L; break; }
        }
      }
    }

    /* Steer toward the target lane centre. PHASE 5: constant-rate lateral
     * motion (road-units/second) means a full lane change takes a fixed,
     * realistic ~1.1–1.6s regardless of frame rate — no snapping, and a
     * gentle S-curve feel comes from the wander sine riding on top. */
    const targetX = laneCentre(car.targetLane)
                  + Math.sin(car.z * 0.7 + car.aggression * 9) * car.wander;
    const dxLane = targetX - car.x;
    const laneRate = 0.85 + car.aggression * 0.65;  // road-units/sec
    const maxStep = laneRate * dt;
    car.x += Math.sign(dxLane) * Math.min(Math.abs(dxLane), maxStep);
    car._laneV = Math.sign(dxLane) * Math.min(Math.abs(dxLane) / Math.max(dt, 1e-4), laneRate);

    // Legacy gentle wander retained (bounded to the tarmac).
    car.x += car.drift * dt * 0.5;
    if (car.x < -0.95 || car.x > 0.95) car.drift *= -1;
    car.x = Math.max(-0.95, Math.min(0.95, car.x));

    // Relative position: how far ahead of the player (in segments)?
    let rel = car.z - playerSeg;
    if (rel < -g.totalSegs / 2) rel += g.totalSegs;
    if (rel > g.totalSegs / 2) rel -= g.totalSegs;

    /* PHASE 5 · traffic recycling. Once a car falls well behind the
     * player it is re-emitted as a FRESH car (new style, lane, colour,
     * personality). Without this the pack thins out permanently as the
     * player outpaces it — the road goes empty by lap two.
     *
     * P3.1/P3.2/P3.5 · re-emission rules:
     *   • only at the FAR END of the draw distance (≥ DRAW_DIST-20 segs
     *     ahead) — nothing ever appears in the near field;
     *   • never in the player's own lane within the visible window
     *     (no instant obstacle in front of the player);
     *   • the new slot must leave one lane free in its ~15-seg group;
     *   • the car enters with a fresh 0.4s fade/scale-in window, so even
     *     a worst-case pop is a soft ease-in rather than a hard appear.
     * Placement is OCCUPANCY-AWARE via the shared probe. */
    if (rel < -10) {
      const laneW2 = 2 / CR.LANES;
      let placed = false;
      for (let attempt = 0; attempt < 10 && !placed; attempt++) {
        /* P3.1 · far-end-only: candidates start at DRAW_DIST-20 and go
         * further out; the +attempt*16 keeps probing outward. */
        const zTry = (playerSeg + (CR.DRAW_DIST - 20)
                      + Math.random() * 20 + attempt * 16) % g.totalSegs;
        const laneTry = Math.floor(Math.random() * CR.LANES);
        const xTry = -1 + laneW2 * (laneTry + 0.5);
        /* P3.6 · never directly in front of the player in the player's
         * lane: if this candidate would land in the player's lane within
         * the visible field, shift it one lane over. */
        let laneOk = laneTry;
        if (Math.abs(xTry - g.playerX) < laneW2 * 0.7) {
          laneOk = (laneTry + 1) % CR.LANES;
        }
        const xOk = -1 + laneW2 * (laneOk + 0.5);
        /* BUGFIX 7 · shared occupancy probe (excludes `car` itself). */
        const others = g.traffic.filter(o => o !== car);
        let clear = crTrafficSlotFree(zTry, xOk, others);
        /* P3.5 · one lane free per spawn group: count occupants of the
         * ~15-segment group around the candidate; reject when all but
         * one lane would be taken. */
        if (clear) {
          let group = 0;
          for (const o of others) {
            let rz = o.z - zTry;
            if (rz < -g.totalSegs / 2) rz += g.totalSegs;
            if (rz > g.totalSegs / 2) rz -= g.totalSegs;
            if (Math.abs(rz) < 15) group++;
          }
          if (group >= CR.LANES - 1) clear = false;
        }
        if (clear) {
          const fresh = crMakeCar();
          for (const k of ['speed','color','w','h','style','drift',
                           'aggression','speedBias','wander','reaction']) {
            car[k] = fresh[k];
          }
          car.z = zTry; car.x = xOk;
          car.lane = laneOk; car.targetLane = laneOk;
          car.passedFlag = false; car.nearMissed = false;
          car.bumped = 0; car.laneT = 0;
          car.spawnT = 0.4;                       // P3.2 · ease into view
          placed = true;
        }
      }
      rel = car.z - playerSeg;
      if (rel < -g.totalSegs / 2) rel += g.totalSegs;
      if (rel > g.totalSegs / 2) rel -= g.totalSegs;
    }

    // Overtake scoring: the car passes behind us.
    if (!car.passedFlag && rel < -1.2 && rel > -6) {
      car.passedFlag = true;
      g.passed++;
      const bonus = car.nearMissed ? 120 : 55;
      g.score += bonus;
      if (car.nearMissed) { crPopup('NEAR MISS +120', '#5ef2c0'); crSfx('nearmiss'); }
      else crSfx('pass');
      /* BUGFIX 4 · overtakes charge nitro (skill -> boost). The award
       * rides the SAME once-per-approach flag as the score bonus, so a
       * single pass can never pay twice; capped at the meter maximum. */
      g.nitro = Math.min(1, g.nitro + (car.nearMissed ? P.NITRO_GAIN_MISS : P.NITRO_GAIN_PASS));
      car.nearMissed = false;
    }
    /* BUGFIX 6 · shared contact half-widths (declared at first use):
     * bodyW * CAR_BODY_K * CAR_VIS_K for both parties — the identical
     * chain the renderer draws, so physics and pixels always agree. */
    const carHalf = car.w * P.CAR_BODY_K * CR.CAR_VIS_K;
    const playerHalf = P.PLAYER_WIDTH * P.CAR_BODY_K * CR.CAR_VIS_K;
    // Near miss: we pass close alongside (within z-range, just outside
    // collision overlap). Detected once per approach; pays out on overtake.
    if (!car.nearMissed && rel > -0.9 && rel < 1.2) {
      const dx = Math.abs(car.x - g.playerX);
      const hitW = carHalf + playerHalf;
      if (dx >= hitW && dx < hitW + 0.30 && g.speed > g.maxSpeed * 0.45) {
        car.nearMissed = true;
        g.nearMiss = 0.5;                 // HUD flash
        g.nearMisses++;                   // PHASE 4 telemetry
        // PHASE 6 · Time Attack: bravery buys time.
        if (g.mode === 'timeattack') g.timeLeftMs += 1200;
      }
    }
    // Reset the flag once the car is far ahead again (next lap).
    if (car.passedFlag && rel > 12) car.passedFlag = false;

    // Collision: overlapping z-range and lateral overlap.
    /* BUGFIX 6 · lateral threshold from the SHARED visual-width chain:
     * half-widths are bodyW * CAR_BODY_K * CAR_VIS_K for both parties,
     * exactly what the renderer draws at that depth. Declared BEFORE the
     * near-miss block above (both use the same halves). */
    if (rel > -0.9 && rel < 1.5) {
      const dx = Math.abs(car.x - g.playerX);
      if (dx < carHalf + playerHalf) {
        // While contact persists the grace window keeps refreshing, so a
        // single continuous scrape is ONE penalty — not one per second.
        if (g.crashGrace > 0) { g.crashGrace = Math.max(g.crashGrace, P.COLLISION_GRACE_HOLD); }
        else crCrash(car);
      }
    }
  }

  /* BUGFIX 7 · pairwise separation pass (end of tick). Any two cars that
   * ended up closer than MIN_GAP_SEGS while laterally adjacent (|dx| <
   * 0.7, covering mid-lane-change positions the leader scan misses) are
   * separated by pushing the trailing car back to the floor. Runs after
   * every car has advanced, so the result is the FINAL geometry for the
   * tick — no tunnelling, no same-z stacks. Two sweeps: pushing a car
   * back can place it near a THIRD car, which the second sweep resolves
   * (converges immediately at these densities). O(n²) over ≤50 cars ≈
   * 1200 cheap comparisons per sweep: negligible next to rendering. */
  for (let sweep = 0; sweep < 2; sweep++)
  for (let a = 0; a < g.traffic.length; a++) {
    const A = g.traffic[a];
    for (let b = a + 1; b < g.traffic.length; b++) {
      const B = g.traffic[b];
      if (Math.abs(A.x - B.x) > 0.7) continue;         // not laterally adjacent
      let dz = B.z - A.z;
      if (dz < -g.totalSegs / 2) dz += g.totalSegs;
      if (dz > g.totalSegs / 2) dz -= g.totalSegs;
      if (Math.abs(dz) >= MIN_GAP_SEGS) continue;      // fine already
      // Push the TRAILING car back to the floor (the leader keeps its slot).
      if (dz >= 0) {                                   // B ahead: move A back
        A.z = (A.z - (MIN_GAP_SEGS - dz)) % g.totalSegs;
        if (A.z < 0) A.z += g.totalSegs;
      } else {                                         // A ahead: move B back
        B.z = (B.z - (MIN_GAP_SEGS + dz)) % g.totalSegs;
        if (B.z < 0) B.z += g.totalSegs;
      }
    }
  }
}


/** Resolve a collision. The response is deliberately physical:
 *  • A head-on hit (we drove into them) sheds most speed and shakes hard.
 *  • A side swipe (lateral overlap only) is lighter — we glance off and
 *    keep more momentum, which feels like real body contact.
 *  • The other car is nudged away so we cannot re-collide instantly, and a
 *    short invulnerability window prevents double-penalising one bump. */
function crCrash(car) {
  const g = CRG;
  const P = CRP;
  // Ignore impacts inside the grace window after a previous one.
  if (g.crashGrace > 0) return;

  const preSpeed = g.speed;                  // severity reference (Phase 3)
  const dx = car.x - g.playerX;
  const lateral = Math.abs(dx);
  const headOn = lateral < 0.22;                  // mostly straight behind

  g.crashes++;
  /* PHASE 6 · difficulty forgiveness: Easy softens the score penalty and
   *  the speed loss; Hard does the opposite. The physical response below
   *  (shove, knockback, sparks) is identical on every setting. */
  const D = CR_DIFFS[g.diff] || CR_DIFFS.medium;
  g.combo = 0; g.comboT = 0;
  g.score = Math.max(0, g.score - (headOn ? P.COLLISION_PENALTY_HEAD
                                          : P.COLLISION_PENALTY_SIDE) * D.penaltyK);
  g.nitro = Math.max(0, g.nitro - (headOn ? 0.35 : 0.15));

  // --- speed penalty + feedback ---
  // Phase 3: shake and flash now scale with the closing speed, so a tap at
  // 20% throttle reads completely differently from a full-send T-bone.
  const impactK = Math.min(1, preSpeed / g.maxSpeed);      // 0..1 severity
  if (headOn) {
    g.speed = Math.max(0, g.speed * Math.min(0.95, P.COLLISION_SPEED_HEAD * D.speedKeepK));
    g.shakeT = 0.30 + impactK * 0.34;
    g.flashT = 0.12 + impactK * 0.14;
  } else {
    g.speed = Math.max(0, g.speed * Math.min(0.97, P.COLLISION_SPEED_SIDE * D.speedKeepK));   // glancing blow
    g.shakeT = 0.16 + impactK * 0.22;
    g.flashT = 0.06 + impactK * 0.08;
  }

  // --- Phase 3: brief impact slow-motion ---------------------------------
  // A ~0.18s time dilation on hard hits makes the contact READ. It only
  // scales dt for the simulation step (never the frame loop or UI), and
  // it is skipped entirely under reduced motion.
  if (!g.reducedMotion && headOn && impactK > 0.45) {
    g.slowmo = P.COLLISION_SLOWMO_T;
  }

  // --- collision sparks: a burst of glowing debris at the impact point ---
  // Count also scales with severity now.
  if (!g.reducedMotion) crSparks(Math.round((headOn ? 18 : 10) + impactK * 16), dx);

  // --- controlled knockback: push BOTH parties apart so one bump cannot
  //     re-trigger on the next frame while the geometry still overlaps.
  //     The shove is proportional to the interlock, and the player also
  //     receives a lateral velocity impulse (a real shove, not a warp). ---
  const dir = g.playerX >= car.x ? 1 : -1;        // +1 when we are right of them
  /* BUGFIX 6 · overlap measured with the same shared half-widths as the
   * contact test, so the shove is proportional to the REAL interlock. */
  const overlap = (car.w * P.CAR_BODY_K * CR.CAR_VIS_K
                 + P.PLAYER_WIDTH * P.CAR_BODY_K * CR.CAR_VIS_K) - lateral;
  const shove = Math.max(0.34, overlap + 0.10);   // enough to fully clear
  g.playerX -= dir * shove * 0.42;                // we take the smaller share
  car.x += dir * shove;                           // they take the larger
  // Lateral impulse sells the physical contact (and can start a slide the
  // player must catch — risk/reward for squeezing through gaps).
  g.vlat -= dir * shove * 0.55;
  // The other driver reacts too: wobbles and eases off for a moment.
  car.bumped = 1.2;
  car.x = Math.max(-0.95, Math.min(0.95, car.x)); // keep them on tarmac
  g.playerX = Math.max(-P.BOUND_MAX, Math.min(P.BOUND_MAX, g.playerX));

  // Grace outlasts separation + a reaction beat: one contact, one penalty,
  // even when the player stays boxed in behind the same car.
  g.crashGrace = P.COLLISION_GRACE;
  crSfx('crash');
}


/* =====================================================================
   PHASE 3 · CENTRALIZED PARTICLE ENGINE
   One pooled allocator + one stepper for every screen-space particle
   class. Existing pools (CR_SPARKS, CR_STREAKS, g.smoke) keep their own
   storage — the engine only adds the classes Phase 3 needs, and every
   array is hard-capped so nothing can grow unbounded.
   ===================================================================== */
const CR_FX = {
  /** Exhaust/nitro flame particles (screen-space, anchored to the car). */
  exhaust: [],
  /** Ambient road dust streaking past on the verges (world-anchored). */
  dust:    [],
  caps: { exhaust: 70, dust: 40 },
};
/** Spawn one particle into a capped pool. Returns the slot or null when
 *  the cap is hit — callers must tolerate null (effects degrade, never
 *  leak). Reuses dead slots first so the arrays stay dense. */
function crFxSpawn(pool, p) {
  const arr = CR_FX[pool], cap = CR_FX.caps[pool];
  if (arr.length >= cap) {
    // Recycle the most-dead particle rather than dropping the spawn.
    let oldest = -1, minLife = Infinity;
    for (let i = 0; i < arr.length; i++) {
      if (arr[i].life < minLife) { minLife = arr[i].life; oldest = i; }
    }
    if (minLife < p.life) { arr[oldest] = p; return p; }
    return null;
  }
  arr.push(p);
  return p;
}
/** Advance one pooled class with per-particle velocity/accel/fade.
 *  Allocation-free: mutates in place, compacts with a write index. */
function crFxStep(arr, dt, drag, grav) {
  let w = 0;
  for (let i = 0; i < arr.length; i++) {
    const p = arr[i];
    p.life -= dt * p.decay;
    if (p.life <= 0) continue;
    p.vx = (p.vx + (p.ax || 0) * dt) * drag;
    p.vy = (p.vy + (p.ay || 0) * dt) * drag;
    p.x += p.vx * dt;
    p.y += p.vy * dt;
    p.r += (p.grow || 0) * dt;
    arr[w++] = p;
  }
  arr.length = w;
}
/** Clear every FX pool (restart, teardown, slow-down). */
function crFxClear() {
  CR_FX.exhaust.length = 0;
  CR_FX.dust.length = 0;
}

/** Collision sparks. Screen-space particles with velocity, gravity and a
 *  hot-to-cool colour ramp (white → amber → red → smoke). Fixed pool,
 *  reused — no allocation churn in the loop. */
const CR_SPARKS = [];
function crSparks(n, sideDir) {
  const g = CRG;
  /* FULLSCREEN/VIEWPORT FIX · sparks spawn at the live view centre so
   * they always erupt from the player's screen position. */
  const cx = crViewW() / 2 + (sideDir || 0) * 60;
  const cy = CR.H - 70;
  for (let i = 0; i < n; i++) {
    if (CR_SPARKS.length >= 90) break;            // hard cap
    const ang = -Math.PI / 2 + (Math.random() - 0.5) * 2.4;
    const spd = 120 + Math.random() * 320;
    CR_SPARKS.push({
      x: cx + (Math.random() - 0.5) * 70,
      y: cy + (Math.random() - 0.5) * 24,
      vx: Math.cos(ang) * spd * (sideDir ? Math.sign(sideDir) : (Math.random() - 0.5)),
      vy: Math.sin(ang) * spd,
      life: 0.5 + Math.random() * 0.45,
      r: 1.2 + Math.random() * 2.2,
    });
  }
}

/** Advance + draw spark particles. Called once per frame from crDraw. */
function crSparksStep(c, dt) {
  if (!CR_SPARKS.length) return;
  for (let i = CR_SPARKS.length - 1; i >= 0; i--) {
    const p = CR_SPARKS[i];
    p.life -= dt;
    if (p.life <= 0) { CR_SPARKS.splice(i, 1); continue; }
    p.x += p.vx * dt;
    p.y += p.vy * dt;
    p.vy += 620 * dt;                             // gravity
    p.vx *= 0.985;
  }
  // Draw pass (after physics so positions are current).
  c.save();
  c.globalCompositeOperation = 'lighter';
  for (const p of CR_SPARKS) {
    const k = Math.max(0, Math.min(1, p.life / 0.9));
    if (p.neon) {
      // Phase 3: drift embers glow neon (alternating cyan/pink per index).
      c.fillStyle = (p.x | 0) % 2
        ? `rgba(96,236,255,${(k * 0.85).toFixed(3)})`
        : `rgba(255,120,200,${(k * 0.85).toFixed(3)})`;
    } else {
    // White-hot when fresh, cooling to ember red.
    c.fillStyle = k > 0.66 ? `rgba(255,240,210,${k.toFixed(3)})`
                : k > 0.33 ? `rgba(255,170,80,${k.toFixed(3)})`
                           : `rgba(255,80,60,${(k * 0.8).toFixed(3)})`;
    }
    c.beginPath(); c.arc(p.x, p.y, p.r * (0.6 + k * 0.7), 0, Math.PI * 2); c.fill();
  }
  c.restore();
}


/* PHASE 6 · award achievements for a finished run; returns newly-earned
 * list (duplicates blocked by the stored set). Also refreshes unlocks. */
function crAchievementsCheck(result) {
  const earned = [];
  const ctx = {
    races: CR_SAVE.totals.races + 1,
    maxKmh: result.maxKmh,
    driftTotal: result.driftTotal,
    nearMisses: result.nearMisses,
    crashes: result.crashes,
    distanceKm: result.distanceKm,
    nitroUsedS: result.nitroUsedS,
    mode: result.mode,
  };
  for (const a of CR_ACHIEVEMENTS) {
    if (CR_SAVE.achievements.includes(a.id)) continue;
    let ok = false;
    try { ok = !!a.test(ctx, CR_SAVE); } catch { ok = false; }
    if (ok) { CR_SAVE.achievements.push(a.id); earned.push(a); }
  }
  return earned;
}

/** Update persisted records/totals from a finished run. Returns the list
 *  of record fields that improved (for the results screen). */
function crRecordsUpdate(result) {
  const r = CR_SAVE.records, t = CR_SAVE.totals, imp = [];
  if (result.score > r.highScore) { r.highScore = result.score; imp.push('score'); }
  if (result.distanceKm > r.longestKm) { r.longestKm = +result.distanceKm.toFixed(2); imp.push('distance'); }
  if (result.maxKmh > r.topKmh)       { r.topKmh = result.maxKmh; imp.push('speed'); }
  if (result.bestDrift > r.bestDrift) { r.bestDrift = Math.round(result.bestDrift); imp.push('drift'); }
  if (result.nearMisses > r.mostNear) { r.mostNear = result.nearMisses; imp.push('near'); }
  const cleanKm = result.crashes === 0 ? result.distanceKm : 0;
  if (cleanKm > r.cleanestKm)         { r.cleanestKm = +cleanKm.toFixed(2); imp.push('clean'); }
  /* TASK 7.1 · all-time best lap (only meaningful for completed laps). */
  if (result.bestLapMs > 0 && result.bestLapMs < (r.bestLapMs || Infinity)) {
    r.bestLapMs = Math.round(result.bestLapMs); imp.push('lap');
  }
  // Totals always accumulate.
  t.races++; t.wins += result.won ? 1 : 0;
  t.distanceKm = +(t.distanceKm + result.distanceKm).toFixed(2);
  t.driftTotal += Math.round(result.driftTotal);
  t.nearMisses += result.nearMisses;
  t.nitroUsedS = +(t.nitroUsedS + result.nitroUsedS).toFixed(1);
  t.crashes += result.crashes;
  return imp;
}

/** Fold a race result into an active championship (if any). */
function crChampRecord(result) {
  const ch = CR_SAVE.champ;
  if (!ch || ch.eventIdx >= ch.events.length) return null;
  const ev = ch.events[ch.eventIdx];
  const pts = crChampPoints(result);
  ev.done = true;
  /* TASK 7.2 · store the ACTUAL on-track finishing position alongside
   * the score-derived points, so Task 7.3 can switch championship
   * scoring to real positions without touching recorded data. Rival
   * standings below remain explicitly score-simulated (no invented
   * on-track results) until AI racers exist. */
  ev.result = { score: result.score, rating: result.rating, won: result.won,
                finishPos: result.finishPos || 0, fieldSize: result.fieldSize || 0 };
  ch.points += pts;
  ch.eventIdx++;
  const finished = ch.eventIdx >= ch.events.length;
  if (finished) {
    // AI rivals earn points from the SAME formula on their own simulated
    // event scores (deterministic per event index — no fabricated names).
    const rivals = ['RIVAL-α','RIVAL-β','RIVAL-γ'];
    // 'simulated: true' marks score-derived entries — distinct from any
    // real on-track position (which only the player has today).
    const standings = [{ name:'YOU', pts: ch.points, you:true, simulated:false }];
    for (let i = 0; i < rivals.length; i++) {
      // Rival strength grows with event index; seeded per championship.
      const seed = (ch.startedAt % 997) / 997;
      const base = 900 + i * 140 + seed * 260;
      const pts2 = Math.round((base + ev.target * 0.11 * (i + 1)) / 40 * 1.05);
      standings.push({ name: rivals[i], pts: pts2, simulated:true });
    }
    standings.sort((a,b) => b.pts - a.pts);
    const wonByPlayer = standings[0].you === true;
    CR_SAVE.champHistory.unshift({
      name: new Date(ch.startedAt).toLocaleDateString(),
      standings, wonByPlayer, points: ch.points,
    });
    CR_SAVE.champHistory = CR_SAVE.champHistory.slice(0, 20);
    CR_SAVE.champ = null;
    return { finished, standings, wonByPlayer };
  }
  return { finished, standings: null, wonByPlayer: false };
}

/** End the run and build the result summary. */
function crFinish(won) {
  const g = CRG;
  g.state = 'over';
  g.overAt = performance.now();
  g.speed = 0;
  g.nitroActive = false;
  crEngineStop();
  const isBest = g.score > g.highScore;
  if (isBest) g.highScore = Math.round(g.score);
  /* PHASE 4 · performance rating: a transparent rubric from real stats —
   * speed sustained, clean driving (no crashes), near-miss bravery and
   * drift skill, each contributing up to a quarter of the rating. */
  const speedPts = Math.min(1, (g.maxSpeedHit / g.maxSpeed) - 0.5) / 0.5;
  const cleanPts = g.crashes === 0 ? 1 : Math.max(0, 1 - g.crashes * 0.25);
  const bravePts = Math.min(1, g.nearMisses / 12);
  const driftPts = Math.min(1, g.bestDrift / 600);
  const ratingK = Math.max(0, Math.min(1,
    speedPts * 0.30 + cleanPts * 0.30 + bravePts * 0.15 + driftPts * 0.25));
  const RATING = ['D', 'C', 'B', 'A', 'S'];
  g.result = {
    won,
    score: Math.round(g.score),
    best: isBest,
    distanceKm: g.distance / 100000,      // world units -> display km
    time: g.raceTimeMs,
    passed: g.passed,
    crashes: g.crashes,
    laps: g.laps,
    bestLapMs: g.bestLapMs,
    /* TASK 7.1 · full lap telemetry for the results screen. */
    totalLaps: CR_LAP.enabled ? CR_LAP.totalLaps : 0,
    lapTimes: CR_LAP.lapTimes.slice(),
    prevLapMs: CR_LAP.prevLapMs,
    raceWonByLaps: !!(CR_LAP.enabled && CR_LAP.finished),
    /* TASK 7.2 · actual finishing position from the live standings —
     * never score-derived, never invented. */
    finishPos: (CR_RACERS.length && CR_LAP.enabled)
      ? (CR_RACERS.find(r => r.isPlayer) || { pos: 1 }).pos : 0,
    fieldSize: CR_LAP.enabled ? CR_RACERS.length : 0,
    maxKmh: Math.round(g.maxSpeedHit * CR_KMH),     // PHASE 4
    bestDrift: g.bestDrift,                         // PHASE 4
    nearMisses: g.nearMisses,                       // PHASE 4
    nitroUsedS: g.nitroUsedS,                       // PHASE 4
    rating: RATING[Math.round(ratingK * 4)],        // PHASE 4
    mode: g.mode, diff: g.diff,                    // PHASE 6
    driftTotal: g.driftTotal || 0,                 // PHASE 6 (race total)
  };
  /* PHASE 6 · progression pipeline: records -> achievements -> unlocks ->
   *  championship -> persistence. Runs once per race, all synchronous. */
  const improved = crRecordsUpdate(g.result);
  const newAchievements = crAchievementsCheck(g.result);
  const champ = CRG.champRun ? crChampRecord(g.result) : null;
  g.result.improved = improved;
  g.result.newAchievements = newAchievements;
  g.result.champ = champ;
  /* PHASE 6 · unlock detection: compare unlock state before/after the
   * records update — anything that flipped to unlocked earns a fanfare
   * and is listed on the results screen. */
  const nowUnlocked = [];
  const hadBefore = g._unlockedBefore || [];
  for (const car of CR_CARS)
    if (car.unlock && !hadBefore.includes(car.id) && crUnlocked(car))
      nowUnlocked.push({ kind:'car', name: car.name });
  for (const w of CR_WHEELS)
    if (w.unlock && !hadBefore.includes('w:'+w.id) && crUnlocked(w))
      nowUnlocked.push({ kind:'wheel', name: w.name + ' wheels' });
  for (const s of CR_SPOILERS)
    if (s.unlock && !hadBefore.includes('s:'+s.id) && crUnlocked(s))
      nowUnlocked.push({ kind:'spoiler', name: s.name + ' spoiler' });
  g.result.unlocked = nowUnlocked;
  if (nowUnlocked.length) setTimeout(() => crSfx('unlock'), 650);
  // Best-championship record.
  if (champ && champ.finished && champ.points > 0) { /* points live in save */ }
  crSaveWrite();
  crGarageRefreshLocks();                          // unlock states may have changed
  crSfx(won ? 'win' : 'lose');
  crOverlay('over');
  crHud();
}


/* ---- 6. Rendering ----------------------------------------------------- */
/* FULLSCREEN/VIEWPORT FIX · the live logical view size. Every draw call
 * that positions things horizontally reads this instead of CR.W, so the
 * world fills the actual viewport aspect. Vertical layout (CR.H) is fixed:
 * the road, car and horizon keep identical proportions at every size. */
function crViewW() { return CR.vW; }
/** Project a world point to screen space.
 *  camera-relative depth d, lateral offset x (road units), height y. */
function crProject(d, x, y) {
  /* PHASE 5 · sub-pixel projection. The old Math.round on x/y quantized
   * every projected point to whole pixels: at distance, where a segment
   * moves less than a pixel per frame, that quantization made the road
   * edge "stair-step" and jitter instead of sweeping smoothly toward the
   * horizon. Canvas anti-aliases sub-pixel fills natively, so keeping
   * floats costs nothing visually and removes the shimmer entirely. */
  const fov = (CRG.fov || CR.FOV);
  const camD = 1 / Math.tan((fov / 2) * Math.PI / 180);
  const scale = camD / Math.max(d, 0.01);
  // The camera looks from its own lateral position (chase camera), so the
  // whole road shifts on screen as the player steers — this is what makes
  // cornering readable instead of the car sliding under a frozen road.
  const wx = x - CRG.camX * CR.ROAD_W;
  /* FULLSCREEN/VIEWPORT FIX · horizontal projection uses the LIVE view
   * width (crViewW), so a wider stage shows more world at the same
   * perspective — never a stretched road. Vertical math is unchanged. */
  const VW = crViewW();
  return {
    scale,
    x: VW / 2 + scale * wx * VW / 2,
    y: CR.H / 2 - scale * (y - (CR.CAM_H + (CRG.camY || 0) + (CRG.camBob || 0))) * CR.H / 2,
    w: scale * CR.ROAD_W * VW / 2,
  };
}

/** Draw one frame. All coordinates are in logical units — the live view
 *  width (crViewW()) x CR.H; the canvas transform handles device pixel
 *  ratio and CSS size. */
/** Render the star field once into an offscreen canvas. Deterministic
 *  positions + brightness; reused every frame via drawImage. */
function crBuildStars() {
  /* FULLSCREEN/VIEWPORT FIX · the star field is rebuilt whenever the live
   * logical width changes (crResize invalidates it), so a wider viewport
   * gets a proportionally wider sky with no stretched blit. */
  const VW = crViewW();
  if (CR.starCanvas && CR.starCanvas._vW === VW) return;
  const cv = document.createElement('canvas');
  cv.width = VW; cv.height = Math.ceil(CR.H * 0.42);
  const c = cv.getContext('2d');
  for (let i = 0; i < 150; i++) {
    const x = crHash(i * 3.1) * VW;
    const y = crHash(i * 7.7 + 13) * cv.height * 0.92;
    const b = crHash(i * 11.3 + 5);
    // Dim tiny stars high up, a few brighter ones lower.
    const r = b > 0.93 ? 1.5 : b > 0.75 ? 1.0 : 0.6;
    const a = 0.25 + b * 0.55 * (1 - y / cv.height);
    c.fillStyle = `rgba(226,232,248,${a.toFixed(3)})`;
    c.beginPath(); c.arc(x, y, r, 0, Math.PI * 2); c.fill();
  }
  cv._vW = VW;
  CR.starCanvas = cv;
}

/** Speed streaks: dust motes / light particles that fly past the camera.
 *  Screen-space, spawned just outside the edges, streaking radially inward
 *  with a length proportional to speed. Fixed pool, reused — no per-frame
 *  allocation. This is the cheapest effect with the biggest "velocity"
 *  payoff at 60fps. */
const CR_STREAKS = [];
function crStreaksStep(c, dt) {
  const g = CRG;
  const sr = Math.min(1.25, g.speed / g.maxSpeed);   // can exceed 1 on nitro
  const want = Math.round(4 + sr * 12 + g.nitroVis * 16);
  /* FULLSCREEN/VIEWPORT FIX · the streak field's vanishing point follows
   * the live view width so particles spawn across the whole visible sky,
   * and the cull bound uses the same width (no streaks lingering in a
   * band the player cannot see). */
  const VW = crViewW();
  const cx = VW / 2, cy = H2C;
  // Spawn toward the target count (a couple per frame keeps it smooth).
  let spawn = Math.min(2, want - CR_STREAKS.length);
  while (spawn-- > 0) {
    const ang = Math.random() * Math.PI * 2;
    const r0 = 210 + Math.random() * 150;
    CR_STREAKS.push({
      x: cx + Math.cos(ang) * r0,
      y: cy + Math.sin(ang) * r0,
      dx: Math.cos(ang), dy: Math.sin(ang),
      spd: (260 + Math.random() * 340) * (0.5 + sr * 0.8),
      life: 1,
    });
  }
  c.save();
  if (g.nitroVis > 0.05) c.globalCompositeOperation = 'lighter';
  for (let i = CR_STREAKS.length - 1; i >= 0; i--) {
    const p = CR_STREAKS[i];
    // Move outward from the vanishing point.
    p.x += p.dx * p.spd * dt;
    p.y += p.dy * p.spd * dt;
    // Kill once well off screen.
    if (p.x < -40 || p.x > VW + 40 || p.y < -40 || p.y > CR.H + 40) {
      CR_STREAKS.splice(i, 1); continue;
    }
    // Streak length grows with speed and distance from centre.
    const dist = Math.hypot(p.x - cx, p.y - cy);
    const len = (6 + sr * 26) * Math.min(1.6, dist / 320);
    const a = Math.min(0.5, 0.10 + sr * 0.30);
    c.strokeStyle = g.nitroVis > 0.05
      ? `rgba(190,160,255,${a.toFixed(3)})`
      : `rgba(214,226,246,${(a * 0.8).toFixed(3)})`;
    c.lineWidth = g.nitroVis > 0.05 ? 2 : 1.4;
    c.beginPath();
    c.moveTo(p.x, p.y);
    c.lineTo(p.x - p.dx * len, p.y - p.dy * len);
    c.stroke();
  }
  c.restore();
}
/* Vanishing-point constants for the streak field (logical units).
 * FULLSCREEN/VIEWPORT FIX · W2C is now a function of the live view width
 * (kept for compatibility with any code still reading it). */
const H2C = CR.H * 0.55;
function W2C() { return crViewW() / 2; }

function crDraw() {
  const g = CRG;
  const c = g.ctx; if (!c) return;
  crBuildStars();                      // rebuilds only when the aspect changes
  /* FULLSCREEN/VIEWPORT FIX · W is the LIVE logical width so every draw
   * call below fills the actual viewport. H stays the design height: the
   * road, horizon and car keep identical vertical proportions always. */
  const W = crViewW(), H = CR.H;
  // Frame delta for screen-space particles (clamped; 0 on the first frame).
  const dtDraw = g._dtDraw || 0;

  // Screen shake on impact (skipped under reduced motion). The magnitude
  // is capped so the frame translate can never push HUD instruments past
  // the canvas edge (TASK 7.0 safe-margin companion).
  let sx = 0, sy = 0;
  if (g.shakeT > 0 && !g.reducedMotion) {
    const m = Math.min(9, g.shakeT * 14);
    sx = (Math.random() - 0.5) * m;
    sy = (Math.random() - 0.5) * m;
  }
  c.save();
  c.translate(sx, sy);

  // --- sky + city silhouette ---
  const horizon = H * 0.42;
  const skyGrad = c.createLinearGradient(0, 0, 0, horizon);
  skyGrad.addColorStop(0, '#070a14');      // zenith: near-black indigo
  skyGrad.addColorStop(0.55, '#131033');   // mid: deep violet-blue
  skyGrad.addColorStop(1, '#2a1747');      // horizon: purple haze
  c.fillStyle = skyGrad;
  c.fillRect(-20, -20, W + 40, horizon + 20);

  // Parallax reference: the segment under the camera drives skyline sway.
  /* TASK 7.0 · baseSeg is the segment under the CAMERA (which trails the
   * player), so curve accumulation begins at the true view origin. */
  const baseSeg = g.segs[((Math.floor((g.position - CR.CAM_TRAIL) / CR.SEG_LEN)) % g.totalSegs + g.totalSegs) % g.totalSegs];

  // Stars: fixed field (deterministic), brighter near the top. Rendered
  // once into an offscreen canvas and blitted — zero per-frame cost.
  if (CR.starCanvas) {
    const so = -(g.camX * 6) - baseSeg.curve * 3;
    c.drawImage(CR.starCanvas, Math.round(so * 0.5), 0, W, horizon);
  }

  // Parallax skyline: three layers for depth. Offsets track playerX and
  // road curvature so the world swings through corners.
  /* PHASE 5 · four parallax layers. The new ultra-far layer sits higher
   * and dimmer than the old three, so the city now reads as receding to
   * an actual horizon instead of a single wall of towers. */
  /* BUGFIX (skyline stability) · the skyline OFFSET now carries a
   * persistent world-position term so the city is a fixed object we
   * drive past, plus a gentle sway for corner feel:
   *   • worldPan: distance-driven pan (position/SEG_LEN * k per layer) —
   *     buildings slide by as you drive, exactly like roadside objects.
   *   • curve sway: eased from the CURRENT curve but small, and applied
   *     only as an offset — building identity is hashed on slot index,
   *     not on the offset (see crDrawSkyline), so the city can never
   *     "rebuild itself" when the sway changes through a corner. The old
   *     code hashed towers on (bx + offset) with curve folded into the
   *     offset, so every corner entry morphed the whole skyline. */
  const worldPan = (g.position / CR.SEG_LEN);
  const par = -(g.playerX * 26) - baseSeg.curve * 8;
  const sway = Math.max(-40, Math.min(40, par));
  crDrawSkyline(c, W, horizon, worldPan * 2.0 + sway * 0.18, 0.22, '#0c1120', 148); // ultra-far
  crDrawSkyline(c, W, horizon, worldPan * 3.4 + sway * 0.38, 0.36, '#111729', 118); // far
  crDrawSkyline(c, W, horizon, worldPan * 5.6 + sway * 0.62, 0.56, '#161d33', 92);  // mid
  crDrawSkyline(c, W, horizon, worldPan * 9.0 + sway,      1,    '#212a45', 60);    // near

  /* BUGFIX 8 · atmospheric horizon. The old full-width glow rect ended
   * abruptly at the skyline base and again at the road, reading as a
   * flat solid pink/green band floating between city and tarmac. The
   * replacement is a true fog GRADIENT: alpha eases in from transparent
   * well above the skyline, peaks softly AT the horizon line, and fades
   * back out over the first screen-row of road — so sky, city and
   * tarmac blend continuously. Palette unchanged (violet-magenta haze);
   * nitro still flares it. */
  const nv = g.nitroVis;
  const fogTop = horizon - 46, fogBot = horizon + 26;
  const hg = c.createLinearGradient(0, fogTop, 0, fogBot);
  hg.addColorStop(0, 'rgba(42,23,71,0)');                                   // clear sky
  hg.addColorStop(0.42, `rgba(58,30,88,${(0.34 + nv * 0.10).toFixed(3)})`);  // upper haze
  hg.addColorStop(0.60, `rgba(${Math.round(90 + nv * 40)},${Math.round(52 + nv * 30)},${Math.round(140 + nv * 40)},${(0.38 + nv * 0.12).toFixed(3)})`);
  hg.addColorStop(0.78, `rgba(64,36,96,${(0.26 + nv * 0.08).toFixed(3)})`);  // sinking
  hg.addColorStop(1, 'rgba(20,17,52,0)');                                   // melts into road fog
  c.fillStyle = hg;
  c.fillRect(0, fogTop, W, fogBot - fogTop);

  // Neon sun/moon disc behind the skyline — the OutRun signature. A
  // gradient disc with horizontal scanline gaps that widen downward,
  // drawn BEFORE the skyline so towers occlude it naturally.
  {
    const sx0 = W * 0.72, sy0 = horizon - 44, sr = 66;
    const sg = c.createLinearGradient(0, sy0 - sr, 0, sy0 + sr);
    sg.addColorStop(0, 'rgba(255,140,200,.30)');
    sg.addColorStop(0.55, 'rgba(255,92,168,.24)');
    sg.addColorStop(1, 'rgba(255,60,140,.16)');
    // Disc body via clip so the scanlines cut clean gaps.
    c.save();
    c.beginPath(); c.arc(sx0, sy0, sr, 0, Math.PI * 2); c.clip();
    c.fillStyle = sg;
    c.fillRect(sx0 - sr, sy0 - sr, sr * 2, sr * 2);
    // Scanline gaps: horizontal dark bars, thicker toward the bottom.
    c.fillStyle = 'rgba(7,10,18,.85)';
    for (let gy = sy0 - sr * 0.05; gy < sy0 + sr; ) {
      const k = (gy - sy0) / sr;                   // -1..1 within the disc
      const barH = 2 + Math.max(0, k) * 7;
      c.fillRect(sx0 - sr, gy, sr * 2, barH);
      gy += barH + 5 + Math.max(0, k) * 6;
    }
    c.restore();
    // Soft halo around the sun.
    const halo = c.createRadialGradient(sx0, sy0, sr * 0.4, sx0, sy0, sr * 2.1);
    halo.addColorStop(0, 'rgba(255,92,168,.14)');
    halo.addColorStop(1, 'rgba(255,92,168,0)');
    c.fillStyle = halo;
    c.beginPath(); c.arc(sx0, sy0, sr * 2.1, 0, Math.PI * 2); c.fill();
  }

  // --- road segments (classic pseudo-3D projection) ---
  // Walk forward from the camera segment, accumulating curve into x/dx so
  // bends appear naturally. Each segment's near edge is the previous
  // segment's far edge, which keeps the road continuous.
  /* TASK 7.0 · the camera trails the player by CAM_TRAIL: the walk starts
   * behind the player's world position, so the player's own segment lands
   * at the bottom of the screen and traffic meets it at rel≈0 (where the
   * collision window already fires). The curve accumulator starts from
   * the camera's own segment for continuity. */
  const camZ = g.position - CR.CAM_TRAIL;
  const camSegIdx = Math.floor(camZ / CR.SEG_LEN);
  let maxY = H;
  let x = 0;
  let dx = -(baseSeg.curve * ((camZ % CR.SEG_LEN) / CR.SEG_LEN));

  /* PLAYER-TRACK FIX · curve-offset lookup for EVERY world object.
   * The road polygons bend via the x/dx accumulator below, but sprites
   * (traffic, scenery, player, skids, smoke) projected straight through
   * crProject ignored that accumulation — so in corners cars and lamps
   * floated off the bent road by up to hundreds of pixels. This table
   * records the accumulated road-center offset at each segment ahead of
   * the camera, so any object at depth `n` can add curveX[n] to its own
   * lateral position and land ON the road, in the same coordinate system
   * the tarmac itself uses. Rebuilt every frame with the walk (cheap:
   * it is just the running x value already computed). */
  const curveX = new Float32Array(CR.DRAW_DIST + 2);

  // The near edge of segment 0 is the camera plane itself; project the
  // player's own segment with a small epsilon depth so it never divides
  // by zero and fills the bottom of the screen.
  let prevP = null;
  const visible = [];
  for (let n = 0; n < CR.DRAW_DIST; n++) {
    const idx = ((camSegIdx + n) % g.totalSegs + g.totalSegs) % g.totalSegs;
    const seg = g.segs[idx];
    // Absolute z of this segment's start, unwrapped across laps.
    const segStartAbs = (camSegIdx + n) * CR.SEG_LEN;
    const d1 = Math.max(segStartAbs - camZ, 1);      // clamp at camera plane
    const d2 = Math.max(segStartAbs + CR.SEG_LEN - camZ, 2);

    // Road-center offset at this segment's NEAR edge (world units).
    if (n <= CR.DRAW_DIST + 1) curveX[n] = x;

    const p1 = prevP || crProject(d1, x, seg.y);
    const p2 = crProject(d2, x + dx, seg.y);

    visible.push({ seg, p1, p2, n, d1, d2, x1: x, x2: x + dx });
    prevP = p2;
    x += dx;
    dx += seg.curve;

    // Stop once the road reaches the top of the screen (fully clipped).
    if (p2.y <= 0) break;
    if (p2.y >= maxY && n > 0) continue;
    maxY = p2.y;
  }
  /* Edge values for depths one past the walk (sprites can sit slightly
   * beyond the last drawn segment before their own cull kicks in). */
  curveX[Math.min(CR.DRAW_DIST + 1, visible.length)] = x;
  CR._curveX = curveX;          // consumed by crDrawSprite / crDrawPlayer

  // Draw far-to-near so nearer polygons overwrite farther ones, with a
  // light fog pass on the distant third so the road melts into the night.
  for (let i = visible.length - 1; i >= 0; i--) {
    const v = visible[i];
    const alt = Math.floor(v.n / 3) % 2 === 0;
    crDrawSegment(c, v, alt, W);
    crDrawFog(c, v, W);
  }

  /* ROAD SURFACE FIX · one continuous depth gradient across the whole
   * visible road span. Replaces the per-segment ambient wash + sheen +
   * seam lines that used to stack 8+ flat colors and read as horizontal
   * stripes. A single linear gradient from the player's row (ambient-lit,
   * cool lift) to the horizon row (melting into the violet haze) is drawn
   * over the full-width ground band — so the surface shades smoothly with
   * perspective and consecutive segments are visually ONE plane.
   * Drawn AFTER the segments (so it tints them) but BEFORE the lane
   * markings' re-pass? No: markings are drawn inside crDrawSegment, so
   * this pass must stay TRANSLUCENT enough (≤0.55 at the very top, where
   * fog already dominates) that dashes/edges remain visible — verified:
   * alpha ramps from 0.10 near the camera to 0.55 only in the last 20%,
   * where CR_LIGHT.FOG_MAX already covers the markings. */
  if (visible.length > 1) {
    const vNear = visible[0], vFar = visible[visible.length - 1];
    const yBot = Math.min(H, Math.max(vNear.p1.y, vFar.p1.y));
    const yTop = Math.max(0, vFar.p2.y);
    if (yBot - yTop > 4) {
      const dg = c.createLinearGradient(0, yBot, 0, yTop);
      dg.addColorStop(0, CR_ROAD_SURFACE.GRAD_NEAR);
      dg.addColorStop(0.62, 'rgba(148,170,205,0.035)');
      dg.addColorStop(0.85, 'rgba(38,28,66,0.22)');
      dg.addColorStop(1, CR_ROAD_SURFACE.GRAD_FAR);
      c.fillStyle = dg;
      c.fillRect(0, yTop, W, yBot - yTop);
    }
  }

  // --- sprites (traffic + pylons), far to near ---
  /* FULLSCREEN FIX · menu attract mode draws ONLY the world: road,
   * skyline and roadside scenery. Player car, ordinary traffic, AI
   * racing opponents, checkpoint gates and every race-only instrument
   * are gated on !menu — nothing is deleted, just not drawn while a
   * menu screen is up. Roadside pylons/scenery stay (environmental). */
  const MENU = crMenuMode();
  const playerSegF = g.position / CR.SEG_LEN;
  const sprites = [];
  if (!MENU) for (const car of g.traffic) {
    let rel = car.z - playerSegF;
    if (rel < -g.totalSegs / 2) rel += g.totalSegs;
    if (rel > g.totalSegs / 2) rel -= g.totalSegs;
    /* PHASE 5 · wider sprite window: cars we have just overtaken (rel in
     * -4..0.5) stay visible while they slide down-screen past us, instead
     * of popping out of existence at the moment of passing. */
    if (rel > -4 && rel < CR.DRAW_DIST) sprites.push({ kind: 'car', rel, car });
  }
  /* TASK 7.1 · checkpoint gates: rendered from the live race state so
   * they exist only in lap modes and reflect hit/pending status. */
  if (!MENU && g.cps) {
    for (let ci = 0; ci < g.cps.length; ci++) {
      const cp = g.cps[ci];
      let rel = crWrapSeg(cp.seg - Math.floor(playerSegF), g.totalSegs);
      if (rel > 2 && rel < CR.DRAW_DIST)
        sprites.push({ kind: 'gate', rel, car: null,
                       gateIdx: ci, gateHit: cp.hit,
                       isFinish: ci === 0 });
    }
  }
  /* BUGFIX 3 · AI racing opponents on the road: projected through the
   * same sprite pipeline as traffic (same depth sort, same fade) but
   * tagged 'racer' so the renderer can badge them. Their world position
   * is their own simulated segF (never recycled, never traffic). */
  if (!MENU && CR_LAP.enabled) {
    for (const r of CR_RACERS) {
      if (r.isPlayer || !r.ai || r.finished) continue;
      /* P3.14c · ai.segF may be negative on the grid (behind the line):
       * wrap onto the loop before comparing, then unwrap to the shortest
       * signed distance so same-lap neighbours render at true depth. */
      let rel = (((r.ai.segF % g.totalSegs) + g.totalSegs) % g.totalSegs) - playerSegF;
      // Unwrap across laps: a rival a lap ahead is physically elsewhere
      // on the circuit — show only same-lap neighbours (rel within a lap).
      if (rel < -g.totalSegs / 2) rel += g.totalSegs;
      if (rel > g.totalSegs / 2) rel -= g.totalSegs;
      if (rel > -4 && rel < CR.DRAW_DIST) {
        sprites.push({
          kind: 'car', rel, racer: r,
          car: { x: r.ai.x, w: 0.56, h: 0.50, style: 'sport',
                 color: r.ai.color },
        });
      }
    }
  }
  for (let n = 1; n < CR.DRAW_DIST; n++) {
    const idx = (Math.floor(playerSegF) + n) % g.totalSegs;
    const seg = g.segs[idx];
    for (const sp of seg.sprites) {
      // Pass the sprite's own kind/hue through; cars are handled above.
      sprites.push({ kind: sp.kind, rel: n, side: sp.side,
                     hue: sp.hue, dir: sp.dir });
    }
  }
  sprites.sort((a, b) => b.rel - a.rel);
  /* BUGFIX 6 · the player is part of the SAME depth-sorted sprite pass:
   * cars closer to the camera than the player (rel<0) now correctly
   * occlude her, and cars ahead are correctly occluded BY her. The old
   * fixed "player last" order let the player sprite paint over traffic
   * that was physically in front of the camera.
   * FULLSCREEN FIX · the player car is race-only: skipped entirely while
   * a menu screen is up (attract scene = road + skyline + scenery). */
  const playerSlot = { kind: 'player', rel: 0 };
  let inserted = false;
  if (!MENU) {
    for (let i = 0; i < sprites.length; i++) {
      if (sprites[i].rel < 0) { sprites.splice(i, 0, playerSlot); inserted = true; break; }
    }
    if (!inserted) sprites.push(playerSlot);
  }
  for (const s of sprites) {
    if (s.kind === 'player') crDrawPlayer(c, W, H);
    else crDrawSprite(c, s, W, H);
  }

  // --- skid marks on the road (drawn before the car so tyres sit on them)
  if (!MENU) for (const sk of g.skids) {
    const d = sk.z - g.position;
    if (d < 30 || d > CR.DRAW_DIST * CR.SEG_LEN) continue;
    const seg2 = g.segs[Math.floor(sk.z / CR.SEG_LEN) % g.totalSegs];
    /* PLAYER-TRACK FIX · curve-aware: skids are laid on the bent road. */
    const skRel = Math.round(d / CR.SEG_LEN);
    const skCX = (CR._curveX && skRel >= 0 && skRel <= CR.DRAW_DIST + 1)
               ? CR._curveX[skRel] : 0;
    const p = crProject(d, sk.x * CR.ROAD_W + skCX, seg2.y);
    if (p.y < 0 || p.y > H) continue;
    c.fillStyle = `rgba(10,10,14,${Math.min(0.5, sk.t * 0.22)})`;
    const mw = Math.max(1, p.w * 0.045);
    c.fillRect(p.x - p.w * 0.16, p.y - mw, mw, mw * 1.4);
    c.fillRect(p.x + p.w * 0.16, p.y - mw, mw, mw * 1.4);
  }

  // --- PHASE 4: instrument cluster (canvas corners) ---------------------
  /* FULLSCREEN FIX · every race-only canvas instrument is gated on
   * !MENU so the attract scene shows nothing but the world. */
  if (!MENU) {
    crDrawCluster(c, W, H);

    // --- PHASE 4: opponent radar (top-centre) -----------------------------
    crDrawRadar(c, W, H);

    /* TASK 7.2 · racing position HUD + standings panel (lap modes only;
     * top-left column, clear of the road and the instrument cluster). */
    crDrawPositionHud(c, W, H, dtDraw);
    crDrawStandings(c, W, H);
    /* BUGFIX 10 · race minimap: real track outline + live racer positions
     * (top-right; lap modes only). */
    crDrawMinimap(c, W, H);
  }

  // --- tyre smoke (world-anchored, drifts up and back) -----------------
  if (!MENU) for (const sm of g.smoke) {
    const d = sm.z - g.position;
    if (d < 20) continue;                          // behind the camera
    const segS = g.segs[Math.floor(sm.z / CR.SEG_LEN) % g.totalSegs];
    const age = 1 - sm.life;
    // Phase 3: the puff also carries sideways with the slide (vx), so the
    // plume trails across the road like real tyre smoke.
    const xx = sm.x + (sm.vx ? age * sm.vx * 0.0012 : 0);
    const yy = segS.y + age * sm.vy * 8;           // rises as it ages
    /* PLAYER-TRACK FIX · curve-aware: smoke puffs are emitted at the
     * player's depth, so they must ride the same bent road. */
    const smRel = Math.round(d / CR.SEG_LEN);
    const smCX = (CR._curveX && smRel >= 0 && smRel <= CR.DRAW_DIST + 1)
               ? CR._curveX[smRel] : 0;
    const p = crProject(Math.max(d, 30), xx * CR.ROAD_W + smCX, yy);
    if (p.y < 0 || p.y > H) continue;
    /* BUGFIX (scenery scale family) · smoke radius now derives from the
     * px-per-world-unit chain (p.w / ROAD_W) like every other sprite —
     * the old sm.r * p.scale * 26 drew puffs at ~0.8px (invisible). */
    const r = sm.r * (p.w / CR.ROAD_W) * 0.55;
    if (r < 0.6) continue;
    c.fillStyle = `rgba(190,196,210,${(sm.life * 0.30).toFixed(3)})`;
    c.beginPath(); c.arc(p.x, p.y - r * 0.4, r, 0, Math.PI * 2); c.fill();
  }

  // --- Phase 3: neon tyre sparks while drifting -------------------------
  // Tiny cyan/magenta embers kicked up at the contact patch. They reuse
  // the collision-spark pool (same physics: gravity + drag) but spawn at
  // the rear wheels and burn out fast — a glittering wake, not a firework.
  if (g.drifting && !g.reducedMotion && g.speed > g.maxSpeed * 0.3) {
    g._sparkAcc = (g._sparkAcc || 0) + dtDraw * 46;
    while (g._sparkAcc >= 1) {
      g._sparkAcc -= 1;
      if (CR_SPARKS.length < 90) {
        const side = Math.random() < 0.5 ? -1 : 1;
        CR_SPARKS.push({
          x: W / 2 + side * (36 + Math.random() * 28),
          y: H - 40 + Math.random() * 10,
          vx: -side * (30 + Math.random() * 70),
          vy: -(60 + Math.random() * 150),
          life: 0.22 + Math.random() * 0.2,
          r: 0.9 + Math.random() * 1.5,
          neon: true,                              // tint flag
        });
      }
    }
  }

  // --- speed streaks: dust/motes flying past the camera ----------------
  // Screen-space particles that spawn at the edges and streak inward,
  // lengthening with speed. They sell VELOCITY far better than the static
  // vignette alone. Skipped under reduced motion.
  if ((g.speed > g.maxSpeed * 0.35 || g.nitroActive) && !g.reducedMotion) {
    crStreaksStep(c, dtDraw);
  } else {
    CR_STREAKS.length = 0;                         // clear when slow
  }

  // --- Phase 3: nitro exhaust trail (additive, above the car) ----------
  if (!MENU) crDrawExhaust(c, dtDraw);

  // --- Phase 3: verge dust — world-anchored motes on the shoulders -----
  // Spawned while moving fast; they whip past on both verges and scale
  // with speed, which multiplies the perceived rush without touching the
  // road geometry itself.
  if (!MENU && g.speed > g.maxSpeed * 0.25 && !g.reducedMotion) {
    g._dustAcc = (g._dustAcc || 0) + dtDraw * (10 + 26 * Math.min(1.4, g.speed / g.maxSpeed));
    while (g._dustAcc >= 1) {
      g._dustAcc -= 1;
      const side = Math.random() < 0.5 ? -1 : 1;
      crFxSpawn('dust', {
        z: g.position + CR.SEG_LEN * (14 + Math.random() * 40),
        x: side * (1.22 + Math.random() * 0.34),   // just off the tarmac
        y: 20 + Math.random() * 90,
        r: 1.5 + Math.random() * 2.6,
        life: 1, decay: 0.55,
      });
    }
  }
  if (!MENU && CR_FX.dust.length) {
    let dw = 0;
    for (let i = 0; i < CR_FX.dust.length; i++) {
      const p = CR_FX.dust[i];
      p.life -= dtDraw * p.decay;
      if (p.life <= 0) continue;
      const d = p.z - g.position;
      if (d > 20 && d < CR.DRAW_DIST * CR.SEG_LEN) {
        const segD = g.segs[Math.floor(p.z / CR.SEG_LEN) % g.totalSegs];
        /* PLAYER-TRACK FIX · curve-aware dust motes. */
        const dmRel = Math.round(d / CR.SEG_LEN);
        const dmCX = (CR._curveX && dmRel >= 0 && dmRel <= CR.DRAW_DIST + 1)
                   ? CR._curveX[dmRel] : 0;
        const pr = crProject(d, p.x * CR.ROAD_W + dmCX, segD.y + p.y);
        if (pr.y > 0 && pr.y < H && pr.w > 8) {
          // Motion stretch: the mote is drawn as a short line whose length
          // grows with proximity — reads as motion blur, costs one stroke.
          const len = Math.max(1.5, pr.w * 0.05);
          c.strokeStyle = `rgba(200,214,236,${(p.life * 0.20).toFixed(3)})`;
          /* BUGFIX (scenery scale family) · dust-mote width through the
           * world-unit chain (was p.r * pr.scale * 3 ≈ sub-pixel). */
          c.lineWidth = Math.max(0.7, p.r * (pr.w / CR.ROAD_W) * 2.4);
          c.beginPath();
          c.moveTo(pr.x, pr.y);
          c.lineTo(pr.x, pr.y - len);
          c.stroke();
        }
      }
      CR_FX.dust[dw++] = p;
    }
    CR_FX.dust.length = dw;
  }

  // --- collision sparks --------------------------------------------------
  crSparksStep(c, dtDraw);

  /* TASK 7.1 · race banners: checkpoint confirmation and lap-complete
   * announcements, drawn centre-screen above the traffic. Both reuse the
   * existing eased-timer pattern (alpha from remaining time). */
  if (!MENU && CR_LAP.enabled) {
    const banner = (t, text, col, y) => {
      const a = Math.min(1, t / 0.35) * Math.min(1, t * 2.4);
      c.save();
      c.globalAlpha = a;
      c.textAlign = 'center';
      c.font = '800 34px ui-monospace, monospace';
      c.shadowColor = col; c.shadowBlur = 10;
      c.fillStyle = col;
      c.fillText(text, W / 2, y);
      c.restore();
    };
    if (CR_LAP.lapFlash > 0)
      banner(CR_LAP.lapFlash, CR_LAP.lapFlashText,
             CR_LAP.lapFlashText === 'FINISH!' ? '#5ef2c0' : '#ffc857', H * 0.30);
    else if (CR_LAP.cpFlash > 0)
      banner(CR_LAP.cpFlash, CR_LAP.cpFlashText, '#40e0ff', H * 0.26);
  }

  // --- score popups above the car --------------------------------------
  if (!MENU && g.popups.length) {
    /* BUGFIX 9 · popups stack top-down from a fixed anchor with 26px
     * spacing — simultaneous events can never draw over each other, and
     * the whole stack sits at 66% height so banners (30%) and traffic
     * stay readable. */
    c.save();
    c.textAlign = 'center'; c.textBaseline = 'middle';
    const n = g.popups.length;
    g.popups.forEach((pp, i) => {
      const a = Math.min(1, pp.t * 1.6);
      const rise = (1.15 - pp.t) * 34;
      const slot = -(n - 1 - i) * 26;          // older popups sit higher
      c.globalAlpha = a;
      c.font = '800 17px ui-monospace, monospace';
      c.fillStyle = pp.color;
      c.shadowColor = pp.color; c.shadowBlur = 8;
      c.fillText(pp.text, W / 2 + pp.x, H * 0.66 - rise + slot);
      c.shadowBlur = 0;
    });
    c.globalAlpha = 1;
    c.restore();
  }

  /* BUGFIX 12 · near-miss feedback is now a SUBTLE edge kiss: peak alpha
   * cut from 0.55 to 0.16 and the inner radius pulled in so the centre
   * of the screen (road, vehicles, HUD) stays essentially untinted —
   * the old wash read as a full-screen colour event and stacked badly
   * with lap/checkpoint banners. */
  if (g.nearMiss > 0) {
    const a = g.nearMiss * 0.16;
    const vg = c.createRadialGradient(W / 2, H * 0.55, H * 0.52,
                                      W / 2, H * 0.55, H * 0.98);
    vg.addColorStop(0, 'rgba(94,242,192,0)');
    vg.addColorStop(1, `rgba(94,242,192,${a.toFixed(3)})`);
    c.fillStyle = vg;
    c.fillRect(0, 0, W, H);
  }

  // --- impact / nitro flash + speed lines ---
  if (g.flashT > 0) {
    // Phase 3: white-hot core flash that fades fast — reads as an impact
    // shock rather than a coloured wash.
    const fa = Math.min(0.5, g.flashT * 2.2);
    const fgx = c.createRadialGradient(W / 2, H * 0.72, 40, W / 2, H * 0.72, H * 0.9);
    fgx.addColorStop(0, `rgba(255,246,235,${fa.toFixed(3)})`);
    fgx.addColorStop(1, `rgba(255,170,120,${(fa * 0.35).toFixed(3)})`);
    c.fillStyle = fgx;
    c.fillRect(0, 0, W, H);
  }
  if (g.speed > g.maxSpeed * 0.6 && !g.reducedMotion) {
    crDrawSpeedVignette(c, W, H);
    crDrawPeripheralBlur(c, W, H);                 // Phase 3
  }
  if (g.speed > g.maxSpeed * 0.72 && !g.reducedMotion) {
    crDrawSpeedLines(c, W, H);
  }

  // --- start lights + countdown ---
  // A vertical gantry of three lights that illuminate red-red-red then all
  // switch green on GO — the universal "race start" signal, readable at any
  // size and in any language.
  if (!MENU && g.state === 'countdown' && g.countdown > 0) {
    const t = 3.99 - g.countdown;                  // seconds elapsed
    const lightsOn = Math.min(3, Math.floor(t) + 1);   // 1,2,3 as t grows
    const goPhase = g.countdown <= 1;
    const frac = 1 - (g.countdown % 1);

    // Gantry panel.
    c.save();
    const gw = 150, gh = 210;
    const gx = W / 2 - gw / 2, gy = H * 0.16;
    c.fillStyle = 'rgba(7,9,13,.82)';
    crRoundRect(c, gx, gy, gw, gh, 16);
    c.strokeStyle = goPhase ? '#5ef2c0' : '#ff5ca8';
    c.lineWidth = 2;
    c.stroke();

    for (let L = 0; L < 3; L++) {
      const lx = W / 2;
      const ly = gy + 44 + L * 54;
      const lit = goPhase || L < lightsOn;
      const col = goPhase ? '#5ef2c0' : '#ff5ca8';
      c.beginPath();
      c.arc(lx, ly, 17, 0, Math.PI * 2);
      if (lit) {
        c.fillStyle = col;
        c.shadowColor = col;
        c.shadowBlur = 18;
        c.fill();
        c.shadowBlur = 0;
      } else {
        c.fillStyle = 'rgba(232,236,244,.10)';
        c.fill();
      }
    }
    // Countdown numeral under the gantry.
    c.globalAlpha = Math.min(1, frac * 2);
    c.fillStyle = goPhase ? '#5ef2c0' : '#e8ecf4';
    c.font = '800 64px ui-monospace, monospace';
    c.textAlign = 'center';
    c.textBaseline = 'middle';
    c.fillText(goPhase ? 'GO' : String(4 - lightsOn), W / 2, gy + gh + 46);
    c.globalAlpha = 1;
    c.restore();
  }

  // --- finish flash when the race ends ---
  if (!MENU && g.state === 'over' && g.result) {
    const k = Math.min(1, (performance.now() - (g.overAt || 0)) / 450);
    if (k < 1) {
      c.fillStyle = `rgba(${g.result.won ? '94,242,192' : '255,92,168'},${(1 - k) * 0.22})`;
      c.fillRect(0, 0, W, H);
    }
  }
  c.restore();
}

/** Neon city silhouette (two parallax layers).
 *  Buildings get varied widths, stepped tops, antenna spires and window
 *  grids from a deterministic hash — so the skyline is rich but stable,
 *  never flickering between frames. */
function crDrawSkyline(c, W, horizon, offset, alpha, color, maxH) {
  /* Layered neon city. Every building is deterministic (hash of its x
   * slot), so the skyline is rich but stable — never flickering between
   * frames. Each tower gets:
   *   • a body with a vertical gradient (lighter crown)
   *   • a bright neon EDGE on one side (cyan/magenta mix by layer)
   *   • a window grid with warm/cool lights
   *   • occasional antenna spires with blinking beacons
   *   • occasional horizontal neon signage bands
   * BUGFIX (skyline stability) · towers are hashed on their WORLD SLOT
   * index (slot = floor((bx - offset)/step)), not on the raw screen x +
   * offset. The old hash keyed on (bx + offset): any change to the
   * offset — corner sway, player steering — changed every building's
   * height/width/seed at once, so the city visibly "rebuilt itself"
   * through corners. With slot-keyed hashing the skyline is a fixed
   * city that SLIDES when the offset moves, exactly like a real
   * skyline seen from a moving car. */
  c.save();
  c.globalAlpha = alpha;
  const step = 40;
  const start = Math.floor((-offset) / step) * step - step;
  for (let bx = start; bx < W + step * 2; bx += step) {
    // World slot index for this screen column: stable across offset changes.
    const slot = Math.round((bx - offset) / step);
    const seed = Math.abs(Math.sin(slot * 12.9898) * 43758.5453) % 1;
    const seed2 = Math.abs(Math.sin(slot * 78.233) * 24634.6345) % 1;
    const seed3 = Math.abs(Math.sin(slot * 39.425) * 12345.6789) % 1;
    // Varied width: some towers are slim, some are wide slabs.
    const bw = step * (0.42 + seed2 * 0.44);
    const bxx = bx + offset;
    let bh = 16 + seed * maxH;
    // Stepped top: a taller block beside a shorter one reads as a real
    // skyscraper cluster rather than a fence of equal boxes.
    if (seed2 > 0.62) bh *= 1.28;
    const top = horizon - bh;

    // Body: vertical gradient — darker base, lighter crown catching sky.
    const bg = c.createLinearGradient(0, top, 0, horizon);
    bg.addColorStop(0, color);
    bg.addColorStop(1, '#070a12');
    c.fillStyle = bg;
    c.fillRect(bxx, top, bw, bh + 2);

    // Neon edge light: cyan on the left third of the city, magenta on the
    // right — the whole skyline glows like the road does.
    const hueMix = (bxx + offset) / W;             // 0..1 across the screen
    const edgeCol = hueMix < 0.5
      ? `rgba(64,224,255,${(0.30 + seed3 * 0.25).toFixed(2)})`
      : `rgba(255,92,168,${(0.30 + seed3 * 0.25).toFixed(2)})`;
    c.fillStyle = edgeCol;
    c.fillRect(bxx, top, 1.6, bh + 2);             // left edge
    c.fillRect(bxx + bw - 1.6, top, 1.6, bh + 2);  // right edge
    // Crown cap light.
    c.fillRect(bxx, top, bw, 1.4);

    // Antenna spire on the tall ones.
    if (seed > 0.80) {
      c.fillStyle = '#141b2b';
      c.fillRect(bxx + bw / 2 - 1, top - bh * 0.16, 2, bh * 0.16);
      // Blinking beacon (deterministic phase, no random flicker).
      if (((slot) | 0) % 2 === 0) {
        c.fillStyle = 'rgba(255,92,168,.85)';
        c.beginPath(); c.arc(bxx + bw / 2, top - bh * 0.16, 1.8, 0, Math.PI * 2); c.fill();
      }
    }

    // Window grid: warm and cool lights mixed, density varies per tower.
    if (seed > 0.30 && bh > 26) {
      const cols = Math.max(1, Math.floor(bw / 7));
      const rows = Math.floor((bh - 10) / 8);
      for (let wy = 0; wy < rows; wy++) {
        for (let wx = 0; wx < cols; wx++) {
          const hsh = Math.abs(Math.sin((slot + wx * 13.7 + wy * 7.3) * 91.17) * 4127.3) % 1;
          if (hsh < 0.32) {
            c.fillStyle = hsh < 0.05 ? 'rgba(255,140,200,.60)'
                        : hsh < 0.15 ? 'rgba(180,140,255,.48)'
                                     : 'rgba(111,183,255,.38)';
            c.fillRect(bxx + 3 + wx * 7, top + 5 + wy * 8, 3.2, 3.6);
          }
        }
      }
    }

    // Horizontal neon sign band on some mid-rise towers (a landmark cue
    // that also adds colour variety beyond the window dots).
    if (seed3 > 0.55 && bh > 40 && bw > 18) {
      const sy = top + bh * (0.30 + seed * 0.25);
      const sh = 4 + seed2 * 3;
      const sc = seed2 > 0.5 ? 'rgba(94,242,192,.55)' : 'rgba(255,209,102,.5)';
      c.fillStyle = sc;
      c.fillRect(bxx + 2, sy, bw - 4, sh);
      // Sign "text": darker dashes inside the band.
      c.fillStyle = 'rgba(7,10,16,.75)';
      const dashes = Math.max(2, Math.floor((bw - 8) / 6));
      for (let dI = 0; dI < dashes; dI++) {
        if ((dI + slot) % 3 === 0) continue;
        c.fillRect(bxx + 4 + dI * 6, sy + 1, 3.5, sh - 2);
      }
    }
  }
  c.restore();
}


/** Asphalt palette: four subtle tints, fixed per segment (crHash) so the
 *  surface has believable variation that never shimmers between frames. */
/** Asphalt palette: four subtle tints, fixed per segment (crHash) so the
 *  surface has believable variation that never shimmers between frames.
 *  Two ramps (dark/lit) alternate every 3 segments like real tarmac bands,
 *  which is also what makes the road speed readable.
 *  LIGHTING FIX · raised ~2 stops from the old near-black ramp (#14-18)
 *  into a readable blue-grey (#25-2e). The road now reads as asphalt lit
 *  by ambient city glow instead of a void, while the neon edges keep
 *  their contrast against it. Tint spread widened slightly so the
 *  per-segment surface variation is visible at speed. */
const CR_ASPH = ['#242b39', '#283042', '#222936', '#2a3345'];
const CR_ASPH_LIT = ['#2e374a', '#323c52', '#2c3547', '#343f57'];
/* ROAD SURFACE FIX · one continuous tarmac.
 * The old renderer painted each segment from alt(2) × tint(4) = up to 8
 * different flat shades, then layered a per-segment ambient wash, an
 * alternate-band sheen and seam lines on top. At speed that read as a
 * stack of horizontal rectangles — exactly the "striped road" artifact.
 * The fix keeps TWO alternating shades (the classic pseudo-3D rhythm,
 * needed for the sense of motion) but makes them nearly identical in
 * luminance, drops the per-segment tint/sheen/seam/ambient banding, and
 * replaces all of it with ONE vertical gradient drawn across the whole
 * visible road span (near → horizon) plus a subtle continuous speckle.
 * The result is one continuous surface whose shading follows perspective
 * naturally: bright near the camera, melting into the horizon fog. */
const CR_ROAD_SURFACE = {
  /* The two alternating shades: same hue, ΔL ≈ 1.5% — enough to keep the
   * motion rhythm when driving, invisible as static stripes. */
  BAND_A: '#232a38',
  BAND_B: '#252c3b',
  /* Depth gradient stops (screen-fraction of the road span, color pairs).
   * Drawn once per frame over the full road polygon; blends the tarmac
   * from its near-camera shade into the horizon fog tone. */
  GRAD_NEAR: 'rgba(148,170,205,0.10)',   // ambient lift near the camera
  GRAD_FAR:  'rgba(20,17,52,0.55)',      // melts into the violet haze
};

/* Neon edge palette: the signature synthwave look. Left edge glows cyan
 * (cool), right edge glows magenta (warm) — complementary hues that read
 * instantly at any speed and never blur into the asphalt tint.
 * LIGHTING FIX · same hues, softened channels: the pink drops from full
 * saturation (255,92,168) toward a rose tone so it no longer blooms into
 * a hard hot line; cyan is nudged off pure-blue so both edges sit at a
 * similar perceived brightness. Identity preserved, glare removed. */
const CR_NEON_L = 'rgba(88,214,255,';    // cyan
const CR_NEON_R = 'rgba(240,120,178,';   // soft pink

/* LIGHTING FIX · road-lighting tunables in ONE place (same pattern as
 * CRP): ambient lift, reflection sheen, lamp-pool strength and the neon
 * intensity ceiling. Render code reads these — no magic numbers inline. */
const CR_LIGHT = {
  AMBIENT_NEAR:   0.10,   // cool ambient wash on the nearest third
  AMBIENT_FAR:    0.03,   // eases out with distance (fog takes over)
  SHEEN_ALPHA:    0.05,   // wet-asphalt reflection band (lit segments)
  LANE_ALPHA:     0.85,   // lane-dash visibility (was 0.78)
  EDGE_LINE_ALPHA:0.42,   // solid edge lines (was 0.34)
  NEON_MAX:       0.58,   // edge-line ceiling (was 0.85 — the bloom)
  NEON_GLOW:      0.13,   // outer glow quad (was 0.20)
  RUMBLE_PINK:    '#c94e8f',  // rumble strip (was #ff5ca8 — too hot)
  RUMBLE_SLATE:   '#333c50',  // alternating dark band (lifted to match)
  LAMP_POOL:      0.16,   // tarmac light pool under each street lamp
  FOG_MAX:        0.72,   // distant fog ceiling (was 0.82)
};

/** One road segment: verge, shoulder, barrier, asphalt, lane markers.
 *  Drawn far-to-near by the caller; every polygon shares edges with its
 *  neighbours (p2 of one == p1 of the next), so the road is continuous
 *  with no gaps or z-fighting at any curvature.
 *
 *  Layer order (back to front): verge → guardrail → rumble → shoulder →
 *  asphalt + sheen → centre seam → lane dashes → SOLID NEON EDGES. */
function crDrawSegment(c, v, alt, W) {
  const { p1, p2 } = v;
  const H = CR.H;
  const seg = v.seg;

  // --- verge / off-road ground band -----------------------------------
  /* LIGHTING FIX · verge lifted slightly so the ground beside the road
   * reads as dark terrain catching ambient light — not a black void that
   * makes the tarmac edge look pasted on. Still clearly darker than the
   * shoulder, preserving the road's read.
   * ROAD SURFACE FIX · the verge no longer alternates two shades per
   * segment: that produced the same horizontal striping on the ground
   * beside the road. One flat tone; its depth shading comes from the
   * same full-span gradient pass as the tarmac. */
  c.fillStyle = '#141b28';
  c.fillRect(0, p2.y, W, p1.y - p2.y + 1);

  // Distance-based alpha for edge furniture: full strength near the
  // camera, easing out toward the draw distance so nothing pops in.
  const near = Math.max(0, 1 - v.n / CR.DRAW_DIST);
  const edgeA = 0.35 + 0.65 * near;

  // --- guardrail (outer barrier): post + rail on both verges ----------
  // The rail is a thin quad just outside the rumble strip. Its top edge is
  // projected at RAIL_H world units above the road, which makes it follow
  // hills and curves exactly like the tarmac does.
  /* ROAD SURFACE FIX · CRITICAL: the rail tops were projected at
   * v.x1 ± 1.16 — but v.x1 is in WORLD units, where the road spans
   * ±ROAD_W (±2200). "1.16" therefore meant 1.16 world units from the
   * road CENTER, so every segment drew its rail-face quads as a band
   * straight across the middle of the tarmac. That was the dominant
   * horizontal-striping artifact: alternating bright/dark rows covering
   * the entire road width. The offset must be ±1.16 ROAD WIDTHS
   * (±1.16 × ROAD_W world units), placing the rail just outside the
   * rumble strips where it belongs. */
  const railT1 = crProject(v.d1, v.x1 - 1.16 * CR.ROAD_W, seg.y + CR.RAIL_H);
  const railT2 = crProject(v.d2, v.x2 - 1.16 * CR.ROAD_W, seg.y + CR.RAIL_H);
  const railT3 = crProject(v.d1, v.x1 + 1.16 * CR.ROAD_W, seg.y + CR.RAIL_H);
  const railT4 = crProject(v.d2, v.x2 + 1.16 * CR.ROAD_W, seg.y + CR.RAIL_H);
  const railA = 0.55 + 0.45 * (v.n < 40 ? 1 : Math.max(0, 1 - (v.n - 40) / 60));
  // Left rail face (lit side faces the road).
  c.fillStyle = `rgba(96,116,158,${(railA * 0.75).toFixed(3)})`;
  crPoly(c, p1.x - p1.w * 1.06, p1.y, railT1.x, railT1.y,
         railT2.x, railT2.y, p2.x - p2.w * 1.06, p2.y);
  // Left rail top highlight — now CYAN neon (matches the edge line).
  c.strokeStyle = `${CR_NEON_L}${(railA * 0.6).toFixed(3)})`;
  c.lineWidth = Math.max(1, p1.w * 0.007);
  c.beginPath();
  c.moveTo(railT1.x, railT1.y); c.lineTo(railT2.x, railT2.y);
  c.stroke();
  // Right rail face.
  c.fillStyle = `rgba(96,116,158,${(railA * 0.75).toFixed(3)})`;
  crPoly(c, p1.x + p1.w * 1.06, p1.y, railT3.x, railT3.y,
         railT4.x, railT4.y, p2.x + p2.w * 1.06, p2.y);
  // Right rail top highlight — MAGENTA neon.
  c.strokeStyle = `${CR_NEON_R}${(railA * 0.55).toFixed(3)})`;
  c.lineWidth = Math.max(1, p1.w * 0.007);
  c.beginPath();
  c.moveTo(railT3.x, railT3.y); c.lineTo(railT4.x, railT4.y);
  c.stroke();

  // --- rumble strips (outer coloured edge) ----------------------------
  // Alternating pink / deep-slate bands: the classic OutRun rumble
  // that strobes toward the player and sells speed better than anything.
  // LIGHTING FIX · rumble colours come from CR_LIGHT: the pink band is a
  // rose tone instead of full-brightness magenta, and the dark band is
  // lifted so the alternation stays visible without either extreme.
  const rumble1 = p1.w * 0.14, rumble2 = p2.w * 0.14;
  c.fillStyle = alt ? CR_LIGHT.RUMBLE_SLATE : CR_LIGHT.RUMBLE_PINK;
  crPoly(c, p1.x - p1.w - rumble1, p1.y, p1.x - p1.w, p1.y,
         p2.x - p2.w, p2.y, p2.x - p2.w - rumble2, p2.y);
  crPoly(c, p1.x + p1.w + rumble1, p1.y, p1.x + p1.w, p1.y,
         p2.x + p2.w, p2.y, p2.x + p2.w + rumble2, p2.y);

  // --- shoulder (paved margin inside the rumble) ----------------------
  /* LIGHTING FIX · shoulder lifted in step with the asphalt ramp so the
   * margin stays a shade darker than the running lanes (real motorway
   * look) instead of reading as a black gutter beside them.
   * ROAD SURFACE FIX · single tone (was two alternating shades). */
  c.fillStyle = '#1d2432';
  crPoly(c, p1.x - p1.w, p1.y, p1.x - p1.w * 0.94, p1.y,
         p2.x - p2.w * 0.94, p2.y, p2.x - p2.w, p2.y);
  crPoly(c, p1.x + p1.w * 0.94, p1.y, p1.x + p1.w, p1.y,
         p2.x + p2.w, p2.y, p2.x + p2.w * 0.94, p2.y);

  // --- asphalt: ONE continuous surface ---------------------------------
  /* ROAD SURFACE FIX · the tarmac is now a single continuous plane.
   *   • Only TWO alternating shades, ΔL≈1.5% — the motion rhythm survives
   *     (bands strobe toward you at speed) but as a STATIC image there are
   *     no visible horizontal stripes.
   *   • The per-segment tint variation, ambient wash, reflection sheen and
   *     seam joints are all REMOVED from here: they stacked up to 8+ flat
   *     colors side by side, which is what made the road read as stacked
   *     rectangles. Depth shading is done ONCE per frame by a full-span
   *     gradient (crDrawRoadDepth) after all segments are drawn.
   * Lane dashes, edge lines, rumble strips and neon rails are untouched —
   * they are the intentional markings. */
  c.fillStyle = alt ? CR_ROAD_SURFACE.BAND_B : CR_ROAD_SURFACE.BAND_A;
  crPoly(c, p1.x - p1.w, p1.y, p1.x + p1.w, p1.y,
         p2.x + p2.w, p2.y, p2.x - p2.w, p2.y);

  // --- lane markers: dashes scale correctly toward the horizon --------
  /* LIGHTING FIX · dash alpha/width come from CR_LIGHT so the markings
   *  stay clearly readable against the lifted asphalt — the old 0.78
   *  flat alpha greyed out at distance. Dashes are slightly wider too,
   *  which keeps them visible in the far half where they compress. */
  if (alt) {
    c.fillStyle = `rgba(238,242,250,${CR_LIGHT.LANE_ALPHA})`;
    const lanes = CR.LANES;
    for (let l = 1; l < lanes; l++) {
      const t = -1 + (2 * l) / lanes;
      crPoly(c,
        p1.x + p1.w * t - p1.w * 0.014, p1.y,
        p1.x + p1.w * t + p1.w * 0.014, p1.y,
        p2.x + p2.w * t + p2.w * 0.014, p2.y,
        p2.x + p2.w * t - p2.w * 0.014, p2.y);
    }
  }
  // Solid edge lines just inside the shoulders (real motorway marking).
  if (!alt) {
    c.fillStyle = `rgba(238,242,250,${CR_LIGHT.EDGE_LINE_ALPHA})`;
    for (const t of [-0.90, 0.90]) {
      crPoly(c,
        p1.x + p1.w * t - p1.w * 0.009, p1.y,
        p1.x + p1.w * t + p1.w * 0.009, p1.y,
        p2.x + p2.w * t + p2.w * 0.009, p2.y,
        p2.x + p2.w * t - p2.w * 0.009, p2.y);
    }
  }

  // --- SOLID NEON EDGE LINES (the synthwave signature) -----------------
  // Continuous bright lines just inside each rumble: cyan left, magenta
  // right. Drawn AFTER the asphalt so they stay crisp over every band.
  // Alpha eases out with distance so the far road melts into haze.
  /* LIGHTING FIX · the lines no longer run at near-full alpha: the
   * ceiling comes from CR_LIGHT.NEON_MAX and the outer glow quads are
   * halved, which removes the hard bloom while keeping both hues clearly
   * readable as the track's identity. Nitro still swells them. */
  if (v.n < 110) {
    const lw1 = Math.max(1, p1.w * 0.013);
    const lw2 = Math.max(0.6, p2.w * 0.013);
    // Left (cyan)
    // Phase 3: neon intensity swells while boosting (road lighting).
    const nBoost = 1 + CRG.nitroVis * 0.35;
    c.fillStyle = `${CR_NEON_L}${Math.min(1, CR_LIGHT.NEON_MAX * edgeA * nBoost).toFixed(3)})`;
    crPoly(c,
      p1.x - p1.w * 0.965 - lw1, p1.y,
      p1.x - p1.w * 0.965 + lw1, p1.y,
      p2.x - p2.w * 0.965 + lw2, p2.y,
      p2.x - p2.w * 0.965 - lw2, p2.y);
    // Right (magenta)
    c.fillStyle = `${CR_NEON_R}${Math.min(1, CR_LIGHT.NEON_MAX * edgeA * nBoost).toFixed(3)})`;
    crPoly(c,
      p1.x + p1.w * 0.965 - lw1, p1.y,
      p1.x + p1.w * 0.965 + lw1, p1.y,
      p2.x + p2.w * 0.965 + lw2, p2.y,
      p2.x + p2.w * 0.965 - lw2, p2.y);
    // Soft outer glow on the nearest third only. shadowBlur is expensive
    // per stroke, so the glow is two wider translucent quads instead.
    if (v.n < 46) {
      const g1 = Math.max(1.5, p1.w * 0.030);
      const g2 = Math.max(1, p2.w * 0.030);
      c.fillStyle = `${CR_NEON_L}${Math.min(1, CR_LIGHT.NEON_GLOW * edgeA * nBoost).toFixed(3)})`;
      crPoly(c,
        p1.x - p1.w * 0.965 - g1, p1.y,
        p1.x - p1.w * 0.965 + g1 * 0.4, p1.y,
        p2.x - p2.w * 0.965 + g2 * 0.4, p2.y,
        p2.x - p2.w * 0.965 - g2, p2.y);
      c.fillStyle = `${CR_NEON_R}${Math.min(1, CR_LIGHT.NEON_GLOW * edgeA * nBoost).toFixed(3)})`;
      crPoly(c,
        p1.x + p1.w * 0.965 - g1 * 0.4, p1.y,
        p1.x + p1.w * 0.965 + g1, p1.y,
        p2.x + p2.w * 0.965 + g2, p2.y,
        p2.x + p2.w * 0.965 - g2 * 0.4, p2.y);
    }
  }
}


function crPoly(c, x1, y1, x2, y2, x3, y3, x4, y4) {
  c.beginPath();
  c.moveTo(x1, y1); c.lineTo(x2, y2); c.lineTo(x3, y3); c.lineTo(x4, y4);
  c.closePath(); c.fill();
}

/** Roadside scenery + traffic, projected with the same camera math.
 *  Everything is drawn back-to-front by the caller; each sprite type has
 *  its own cheap vector shape (no images to load, no seams at any scale). */
function crDrawSprite(c, s, W, H) {
  /* TASK 7.0 · depth is measured from the CAMERA, which trails the player
   * by CAM_TRAIL — so rel=0 (the collision centre) lands exactly on the
   * player's screen row. Cars just passed (rel<0) sit between the player
   * and the camera: they fade out rapidly instead of ballooning. */
  const d = s.rel * CR.SEG_LEN + CR.CAM_TRAIL;
  if (d < 60) return;                            // behind/at the camera
  /* TASK 7.0 · negative-safe index: cars just passed (rel<0) at race
   * start made this index negative -> undefined segment -> TypeError.
   * Double-mod keeps it in range for any rel and any position. */
  const segIdx = ((Math.floor(CRG.position / CR.SEG_LEN) + Math.floor(s.rel))
                  % CRG.totalSegs + CRG.totalSegs) % CRG.totalSegs;
  const seg = CRG.segs[segIdx];
  const isCar = s.kind === 'car';
  const lateral = isCar ? s.car.x * CR.ROAD_W : s.side * CR.ROAD_W * 1.35;
  /* PLAYER-TRACK FIX · curve-aware sprite projection: add the road's
   * accumulated center offset at this depth so the object sits ON the
   * bent road, in the same coordinate system as the tarmac polygons.
   * (Before: sprites projected on the straight-ahead axis and visibly
   * detached from the road through corners.) */
  const cx0 = (CR._curveX
            && Math.abs(s.rel) <= CR.DRAW_DIST)
    ? CR._curveX[Math.max(0, Math.min(CR.DRAW_DIST + 1, Math.round(s.rel)))]
    : 0;
  const p = crProject(d, lateral + cx0, seg.y);
  if (p.y < 0 || p.y > H) return;

  /* =================================================================
     BUGFIX (scenery scale) · WORLD-UNIT SPRITE PROJECTION.
     Every roadside object below used to size itself as p.scale * N
     (world units). But p.scale = camD/d is DIMENSIONLESS (~1/2400 at
     mid-field) — it is missing the screen factor that crProject folds
     into p.w (scale × ROAD_W × vW/2). Result: lamps, pylons, trees,
     billboards, gantries and arrows all drew at 0.3–1px — completely
     invisible — leaving an empty road beside a rich skyline. That is
     the single biggest reason the track read as "artificial".
     Fix: one shared converter, u(N) = N world units in pixels, built
     from the SAME chain as traffic cars (p.w / ROAD_W = px per world
     unit), so scenery, cars and the player share one perspective.
     ================================================================= */
  const u = n => n * (p.w / CR.ROAD_W);          // world units -> px

  if (s.kind === 'lamp') {
    // Street lamp: post, arm reaching over the road, glowing head. The
    // pool of light on the tarmac is what makes the road feel lit.
    /* BUGFIX (scenery scale) · dimensions now in real world units through
     * u(): a ~10m lamp next to a ~10m half-width road ≈ 0.75×ROAD_W tall,
     * arm ~0.28×ROAD_W — reads at every depth exactly like traffic does. */
    const h = u(1650);
    const w = Math.max(1, u(52));
    const armLen = u(700) * -s.side;             // reaches toward the road
    c.fillStyle = '#2b3345';
    c.fillRect(p.x - w / 2, p.y - h, w, h);
    c.fillRect(Math.min(p.x, p.x + armLen), p.y - h, Math.abs(armLen) || w, w);
    /* LIGHTING FIX · the lamp now actually LIGHTS THE ROAD: an elliptical
     * warm pool is projected onto the tarmac under the lamp head, fading
     * with distance exactly like every other sprite. This distributes
     * light along the track (the old renderer had a glow around the bulb
     * only, leaving the surface beneath it as dark as everywhere else).
     * One radial gradient per lamp — lamps are ~1 per 7 segments, so at
     * most ~20 gradients in view, and skipped entirely when tiny. */
    if (p.w > 24) {
      const px0 = p.x + armLen * 0.65;
      const poolR = p.w * 0.55;
      if (poolR > 3) {
        const pg = c.createRadialGradient(px0, p.y, 1, px0, p.y, poolR);
        pg.addColorStop(0, `rgba(255,224,160,${CR_LIGHT.LAMP_POOL})`);
        pg.addColorStop(0.6, `rgba(255,214,150,${(CR_LIGHT.LAMP_POOL * 0.45).toFixed(3)})`);
        pg.addColorStop(1, 'rgba(255,214,150,0)');
        c.save();
        // Squash the pool vertically so it reads as light ON the ground
        // plane, not a ball around the bulb.
        c.translate(px0, p.y);
        c.scale(1, 0.32);
        c.translate(-px0, -p.y);
        c.fillStyle = pg;
        c.beginPath(); c.arc(px0, p.y, poolR, 0, Math.PI * 2); c.fill();
        c.restore();
      }
    }
    // Lamp head + glow.
    const hx = p.x + armLen, hy = p.y - h;
    c.fillStyle = '#ffe9a8';
    c.beginPath(); c.arc(hx, hy, Math.max(1.2, u(105)), 0, Math.PI * 2); c.fill();
    if (u(1000) > 3) {
      const gg = c.createRadialGradient(hx, hy, 1, hx, hy, u(1000));
      gg.addColorStop(0, 'rgba(255,222,150,.30)');
      gg.addColorStop(1, 'rgba(255,222,150,0)');
      c.fillStyle = gg;
      c.beginPath(); c.arc(hx, hy, u(1000), 0, Math.PI * 2); c.fill();
    }
    return;
  }
  if (s.kind === 'arrow') {
    // Glowing chevron pointing into the turn, on the outside verge.
    /* BUGFIX (scenery scale) · u()-based: ~0.35×ROAD_W tall chevrons —
     * clearly readable at approach distance, shrinking with perspective. */
    const h = u(780);
    const w = u(560);
    const dir = s.dir || 1;
    c.save();
    c.translate(p.x, p.y - h / 2);
    c.strokeStyle = 'rgba(255,196,90,.92)';
    c.lineWidth = Math.max(1.5, u(74));
    c.lineJoin = 'round'; c.lineCap = 'round';
    c.shadowColor = 'rgba(255,196,90,.8)';
    c.shadowBlur = Math.min(14, u(2600));
    c.beginPath();
    c.moveTo(-w * 0.28 * dir, -h * 0.30);
    c.lineTo(w * 0.24 * dir, 0);
    c.lineTo(-w * 0.28 * dir, h * 0.30);
    c.stroke();
    c.restore();
    return;
  }
  if (s.kind === 'pylon') {
    /* BUGFIX (scenery scale) · u()-based delineator post: slim, ~0.4×
     * ROAD_W tall — a real roadside marker, not a pixel speck. */
    const h = u(900);
    const w = Math.max(1, u(130));
    c.fillStyle = '#ff5ca8';
    c.fillRect(p.x - w / 2, p.y - h, w, h * 0.82);
    c.fillStyle = '#e8ecf4';
    c.fillRect(p.x - w / 2, p.y - h, w, h * 0.2);
    return;
  }
  if (s.kind === 'tree') {
    // Dark conifer: trunk + two stacked triangles.
    /* BUGFIX (scenery scale) · u()-based conifer ~1.2×ROAD_W tall —
     * a background tree that reads at mid-distance. */
    const h = u(2600);
    const w = u(900);
    c.fillStyle = '#1a1410';
    c.fillRect(p.x - w * 0.06, p.y - h * 0.3, w * 0.12, h * 0.3);
    c.fillStyle = '#12241c';
    crPoly(c, p.x - w * 0.5, p.y - h * 0.28, p.x + w * 0.5, p.y - h * 0.28,
           p.x, p.y - h * 0.72);
    c.fillStyle = '#163024';
    crPoly(c, p.x - w * 0.38, p.y - h * 0.58, p.x + w * 0.38, p.y - h * 0.58,
           p.x, p.y - h);
    return;
  }
  if (s.kind === 'gantry') {
    /* Full-width overhead sign bridge: two lattice legs + truss beam +
     * a green direction panel with route bars. Scaled purely by the
     * projection so it grows and sweeps overhead exactly like the road.
     * BUGFIX (scenery scale) · leg height now u()-based (~1.55×ROAD_W —
     * clears trucks under the beam); the span already used p.w (correct). */
    const legH = u(3400);
    const spanW = p.w * 2.5;                       // wider than the road
    const legW = Math.max(1.5, u(150));
    c.fillStyle = '#1b2231';
    // Legs (just outside each rumble).
    c.fillRect(p.x - spanW / 2, p.y - legH, legW, legH);
    c.fillRect(p.x + spanW / 2 - legW, p.y - legH, legW, legH);
    // Truss beam across the top.
    c.fillRect(p.x - spanW / 2, p.y - legH, spanW, legW * 1.5);
    // Lattice X bracing on the beam (only when near enough to see it).
    if (legW > 3) {
      c.strokeStyle = 'rgba(120,140,180,.35)';
      c.lineWidth = 1;
      const nBr = 6, bw = spanW / nBr;
      for (let b = 0; b < nBr; b++) {
        c.beginPath();
        c.moveTo(p.x - spanW / 2 + b * bw, p.y - legH);
        c.lineTo(p.x - spanW / 2 + (b + 1) * bw, p.y - legH + legW * 1.5);
        c.stroke();
      }
    }
    // Direction panel (dark green with light route bars).
    const pw = spanW * 0.42, ph = legH * 0.16;
    c.fillStyle = '#0d2418';
    c.fillRect(p.x - pw / 2, p.y - legH + legW * 1.9, pw, ph);
    c.strokeStyle = 'rgba(120,255,200,.4)';
    c.lineWidth = Math.max(0.8, u(260));
    c.strokeRect(p.x - pw / 2, p.y - legH + legW * 1.9, pw, ph);
    c.fillStyle = 'rgba(190,255,225,.75)';
    const bars = 2, bbw = pw * 0.55 / bars;
    for (let b = 0; b < bars; b++) {
      c.fillRect(p.x - pw * 0.32 + b * 2 * bbw,
                 p.y - legH + legW * 1.9 + ph * (0.28 + b * 0.36), bbw, ph * 0.2);
    }
    return;
  }
  if (s.kind === 'gate') {
    /* TASK 7.1 · checkpoint gate: two neon pylons + a beam across the
     * road, coloured by state — cyan pending, green when this lap's gate
     * is already crossed, gold for the start/finish line. Scales with the
     * same projection as everything else.
     * BUGFIX (scenery scale) · pylon height u()-based (~0.68×ROAD_W). */
    const h = u(1500);
    const legW = Math.max(1.5, u(60));
    const spanW = p.w * 2.05;
    const col = s.isFinish ? '#ffc857' : (s.gateHit ? '#5ef2c0' : '#40e0ff');
    c.save();
    // Pylons.
    c.fillStyle = '#1b2231';
    c.fillRect(p.x - spanW / 2, p.y - h, legW, h);
    c.fillRect(p.x + spanW / 2 - legW, p.y - h, legW, h);
    // Cross beam with glow.
    c.shadowColor = col;
    c.shadowBlur = Math.min(16, u(3000));
    c.strokeStyle = col;
    c.lineWidth = Math.max(1.5, u(110));
    c.beginPath();
    c.moveTo(p.x - spanW / 2, p.y - h);
    c.lineTo(p.x + spanW / 2, p.y - h);
    c.stroke();
    c.shadowBlur = 0;
    // Small marker lights down each pylon (visible when near).
    if (legW > 2.5) {
      c.fillStyle = col;
      for (let k = 1; k <= 4; k++) {
        c.fillRect(p.x - spanW / 2, p.y - h * (k / 5), legW, legW * 0.6);
        c.fillRect(p.x + spanW / 2 - legW, p.y - h * (k / 5), legW, legW * 0.6);
      }
    }
    c.restore();
    return;
  }
  if (s.kind === 'billboard') {
    // Neon hoarding on two legs — a landmark that sells speed as it grows.
    /* BUGFIX (scenery scale) · u()-based sizing.
     * VISUAL COMPOSITION FIX · billboards reduced to ~55% of their first
     * restored size and pushed further out on the verge (the sprite's
     * lateral placement already sits at 1.35×ROAD_W; the panel now leans
     * AWAY from the road by its own half-width so it never overhangs the
     * driving lanes). At 2300u tall the hoardings towered over the road
     * and blocked the view into corners; at 1250u they read as roadside
     * signage beside the track, not above it. */
    const h = u(1250);
    const w = u(1650);
    const legW = Math.max(1, u(70));
    // Shift the whole board outward so its inner edge stays off the tarmac.
    const out = s.side < 0 ? -w * 0.12 : w * 0.12;
    c.fillStyle = '#232b3a';
    c.fillRect(p.x - w * 0.3 + out, p.y - h * 0.55, legW, h * 0.55);
    c.fillRect(p.x + w * 0.3 + out - legW, p.y - h * 0.55, legW, h * 0.55);
    const hue = s.hue || 300;
    c.fillStyle = `hsl(${hue} 60% 16%)`;
    c.fillRect(p.x - w / 2 + out, p.y - h, w, h * 0.5);
    c.strokeStyle = `hsl(${hue} 85% 62%)`;
    c.lineWidth = Math.max(1, u(40));
    c.strokeRect(p.x - w / 2 + out, p.y - h, w, h * 0.5);
    // Simple bar motif instead of text (text at distance shimmers).
    c.fillStyle = `hsl(${hue} 90% 70%)`;
    const bars = 3, bw = w * 0.6 / (bars * 2 - 1);
    for (let b = 0; b < bars; b++) {
      c.fillRect(p.x - w * 0.3 + out + b * 2 * bw, p.y - h * 0.78, bw, h * 0.26);
    }
    return;
  }

  /* =================================================================
     PHASE 5 · TRAFFIC CAR RENDERER — five detailed variants.
     Every car gets: ground shadow, wheels with visible rims, bumpers,
     rear window, tail lights with glow, plate and depth-scaled detail.
     The old "ghost car" bug is fixed at the source: alpha now depends on
     how far the car actually is (n = segments ahead), not on p.scale*9
     which was ~0.001 for everything and rendered all cars at 45%.
     ================================================================= */
  /* TASK 7.0 · uniform vehicle scale: every traffic body is drawn at the
   * same fraction of its road-unit width, so hatch/sedan/van/truck keep
   * their relative sizes at ANY depth and match the player's proportion. */
  const carH = s.car.h || 0.64;
  /* P3.2 · spawn ease-in: while the fresh-car window is open the body is
   * scaled up from 55% to full and fades in — a recycled car reads as
   * driving out of the haze instead of teleporting into place. */
  const spawnK = s.car.spawnT > 0 ? (1 - s.car.spawnT / 0.4) : 1;
  const ease = spawnK < 1 ? (0.55 + 0.45 * spawnK) : 1;
  const cw = p.w * 0.56 * s.car.w * CR.CAR_VIS_K * ease;
  const ch = cw * carH;
  const cx = p.x, cy = p.y;
  if (cw < 1.2) return;                            // too far to draw

  // Depth fade: full opacity until 60 segments out, then a gentle ease
  // into the haze so distant cars melt rather than pop.
  let depthA = Math.max(0.55, Math.min(1, 1 - (s.rel - 60) / 90));
  /* TASK 7.0 · passed-car fade: once a car is behind the player's plane
   * (rel<0) it is between the player and the camera — physically it has
   * been overtaken. Fade it over ~1 segment so it never renders as a
   * giant clipped blob at the frame edge. */
  if (s.rel < 0) depthA *= Math.max(0, 1 + s.rel / 1.0);
  if (spawnK < 1) depthA *= Math.max(0, spawnK);   // P3.2 · alpha ease-in
  c.save();
  c.globalAlpha = depthA;

  const style = s.car.style || 'sedan';
  // Wheel geometry shared by all styles (proportional to body).
  const wheelR = ch * (style === 'truck' || style === 'van' ? 0.16 : 0.20);
  const trackW = cw * 0.40;                        // half-distance between wheels

  // --- ground shadow: soft ellipse under the whole body ---------------
  c.fillStyle = 'rgba(0,0,0,.42)';
  c.beginPath();
  c.ellipse(cx, cy - ch * 0.04, cw * 0.52, ch * 0.13, 0, 0, Math.PI * 2);
  c.fill();

  // --- rear wheels (drawn before the body so arches overlap them) -----
  const wy = cy - wheelR * 0.9;
  crDrawTrafficWheel(c, cx - trackW, wy, wheelR);
  crDrawTrafficWheel(c, cx + trackW, wy, wheelR);

  /* BUGFIX 3 · racing-opponent badge: a small position tag + neon outline
   * floats above AI rivals so they read as COMPETITORS, not traffic.
   * P3.15 · the tag now shows NAME + position and scales with distance
   * (cw-proportional font, clamped to a readable band) so near rivals
   * read clearly while far ones shrink instead of shimmering. */
  if (s.racer) {
    const tagY = cy - ch - Math.max(7, cw * 0.16);
    c.save();
    const fs = Math.max(8, Math.min(17, Math.round(cw * 0.14)));
    c.font = `800 ${fs}px ui-monospace, monospace`;
    c.textAlign = 'center'; c.textBaseline = 'alphabetic';
    c.shadowColor = '#40e0ff'; c.shadowBlur = 6;
    c.fillStyle = '#40e0ff';
    c.fillText(`P${s.racer.pos} ${s.racer.name}`, cx, tagY);
    c.restore();
    // Neon underglow ring — the rival signature.
    c.strokeStyle = 'rgba(64,224,255,.5)';
    c.lineWidth = Math.max(1, cw * 0.02);
    c.beginPath();
    c.ellipse(cx, cy - ch * 0.02, cw * 0.55, ch * 0.15, 0, 0, Math.PI * 2);
    c.stroke();
  }

  const col = s.car.color;
  // Body vertical gradient: roof catches sky light, flanks fall dark.
  let bg = c.createLinearGradient(0, cy - ch, 0, cy);
  bg.addColorStop(0, col);
  bg.addColorStop(0.62, col);
  bg.addColorStop(1, 'rgba(6,9,15,.78)');

  if (style === 'truck') {
    /* --- TRUCK: high-sided box rig with roller door + marker bar --- */
    c.fillStyle = bg;
    crRoundRect(c, cx - cw / 2, cy - ch, cw, ch, cw * 0.03);
    // Roller shutter: horizontal slats.
    c.fillStyle = 'rgba(8,11,18,.34)';
    for (let i = 0; i < 6; i++) {
      c.fillRect(cx - cw * 0.40, cy - ch * 0.86 + i * ch * 0.115,
                 cw * 0.80, ch * 0.035);
    }
    // Roof marker lights (amber, three across).
    if (cw > 10) {
      c.fillStyle = 'rgba(255,196,90,.95)';
      for (let m = -1; m <= 1; m++) {
        c.beginPath();
        c.arc(cx + m * cw * 0.22, cy - ch * 0.94, Math.max(0.7, cw * 0.018), 0, Math.PI * 2);
        c.fill();
      }
    }
    // Bumper bar (chunky steel).
    c.fillStyle = '#2a3242';
    c.fillRect(cx - cw * 0.47, cy - ch * 0.17, cw * 0.94, ch * 0.15);
    c.fillStyle = 'rgba(200,215,235,.18)';
    c.fillRect(cx - cw * 0.47, cy - ch * 0.17, cw * 0.94, ch * 0.03);
    // Mudflaps behind the wheels.
    c.fillStyle = 'rgba(5,8,12,.8)';
    c.fillRect(cx - trackW - wheelR * 0.9, cy - ch * 0.10, wheelR * 1.8, ch * 0.10);
    c.fillRect(cx + trackW - wheelR * 0.9, cy - ch * 0.10, wheelR * 1.8, ch * 0.10);
  } else if (style === 'van') {
    /* --- VAN: tall box with split doors + roof markers --- */
    c.fillStyle = bg;
    crRoundRect(c, cx - cw / 2, cy - ch, cw, ch, cw * 0.07);
    // Split rear doors.
    c.fillStyle = 'rgba(8,11,18,.45)';
    c.fillRect(cx - cw * 0.012, cy - ch * 0.92, cw * 0.024, ch * 0.76);
    // High rear window band.
    c.fillStyle = 'rgba(9,13,22,.85)';
    crRoundRect(c, cx - cw * 0.34, cy - ch * 0.88, cw * 0.68, ch * 0.26, cw * 0.04);
    c.fillStyle = 'rgba(140,190,255,.12)';
    c.fillRect(cx - cw * 0.31, cy - ch * 0.86, cw * 0.62, ch * 0.05);
    // Roof marker lamps.
    if (cw > 12) {
      c.fillStyle = 'rgba(255,196,90,.9)';
      for (let m = -1; m <= 1; m++) {
        c.beginPath();
        c.arc(cx + m * cw * 0.16, cy - ch * 0.955, Math.max(0.7, cw * 0.016), 0, Math.PI * 2);
        c.fill();
      }
    }
    // Bumper + plate.
    c.fillStyle = 'rgba(10,13,20,.72)';
    c.fillRect(cx - cw * 0.46, cy - ch * 0.16, cw * 0.92, ch * 0.14);
    c.fillStyle = 'rgba(210,220,235,.75)';
    c.fillRect(cx - cw * 0.09, cy - ch * 0.145, cw * 0.18, ch * 0.075);
  } else if (style === 'sport') {
    /* --- SPORTS: low wedge, wing, diffuser, twin exhausts --- */
    c.fillStyle = bg;
    c.beginPath();
    c.moveTo(cx - cw / 2, cy - ch * 0.16);
    c.lineTo(cx - cw * 0.44, cy - ch * 0.56);
    c.quadraticCurveTo(cx, cy - ch * 0.66, cx + cw * 0.44, cy - ch * 0.56);
    c.lineTo(cx + cw / 2, cy - ch * 0.16);
    c.quadraticCurveTo(cx + cw * 0.46, cy - ch * 0.04, cx + cw * 0.34, cy - ch * 0.03);
    c.lineTo(cx - cw * 0.34, cy - ch * 0.03);
    c.quadraticCurveTo(cx - cw * 0.46, cy - ch * 0.04, cx - cw / 2, cy - ch * 0.16);
    c.closePath(); c.fill();
    // Raked cabin glass.
    c.fillStyle = 'rgba(9,13,22,.9)';
    c.beginPath();
    c.moveTo(cx - cw * 0.25, cy - ch * 0.54);
    c.quadraticCurveTo(cx, cy - ch * 0.97, cx + cw * 0.25, cy - ch * 0.54);
    c.closePath(); c.fill();
    c.fillStyle = 'rgba(150,200,255,.16)';
    c.fillRect(cx - cw * 0.20, cy - ch * 0.66, cw * 0.40, ch * 0.06);
    // Rear wing with endplates.
    c.fillStyle = 'rgba(12,16,24,.95)';
    c.fillRect(cx - cw * 0.45, cy - ch * 1.00, cw * 0.90, ch * 0.09);
    c.fillRect(cx - cw * 0.45, cy - ch * 1.02, cw * 0.035, ch * 0.11);
    c.fillRect(cx + cw * 0.415, cy - ch * 1.02, cw * 0.035, ch * 0.11);
    c.fillStyle = 'rgba(160,200,255,.22)';
    c.fillRect(cx - cw * 0.45, cy - ch * 1.00, cw * 0.90, ch * 0.025);
    // Diffuser fins + twin pipes.
    c.fillStyle = 'rgba(8,11,18,.8)';
    c.fillRect(cx - cw * 0.33, cy - ch * 0.11, cw * 0.66, ch * 0.09);
    c.fillStyle = '#3c4a63';
    c.beginPath(); c.arc(cx - cw * 0.20, cy - ch * 0.06, cw * 0.030, 0, Math.PI * 2); c.fill();
    c.beginPath(); c.arc(cx + cw * 0.20, cy - ch * 0.06, cw * 0.030, 0, Math.PI * 2); c.fill();
    // Plate recess.
    c.fillStyle = 'rgba(210,220,235,.7)';
    c.fillRect(cx - cw * 0.08, cy - ch * 0.26, cw * 0.16, ch * 0.09);
  } else if (style === 'hatch') {
    /* --- HATCH: compact, near-vertical tailgate --- */
    c.fillStyle = bg;
    crRoundRect(c, cx - cw / 2, cy - ch * 0.82, cw, ch * 0.82, cw * 0.10);
    // Tailgate seam + upright glasshouse.
    c.fillStyle = 'rgba(8,11,18,.35)';
    c.fillRect(cx - cw * 0.36, cy - ch * 0.78, cw * 0.72, ch * 0.02);
    c.fillStyle = 'rgba(9,13,22,.88)';
    crRoundRect(c, cx - cw * 0.30, cy - ch * 1.00, cw * 0.60, ch * 0.30, cw * 0.07);
    c.fillStyle = 'rgba(140,190,255,.13)';
    c.fillRect(cx - cw * 0.26, cy - ch * 0.97, cw * 0.52, ch * 0.055);
    // Roof rail highlight.
    c.fillStyle = 'rgba(255,255,255,.14)';
    c.fillRect(cx - cw * 0.28, cy - ch * 1.01, cw * 0.56, ch * 0.02);
    // Bumper + plate.
    c.fillStyle = 'rgba(10,13,20,.66)';
    c.fillRect(cx - cw * 0.44, cy - ch * 0.17, cw * 0.88, ch * 0.15);
    c.fillStyle = 'rgba(210,220,235,.72)';
    c.fillRect(cx - cw * 0.08, cy - ch * 0.155, cw * 0.16, ch * 0.08);
  } else {
    /* --- SEDAN: three-box with boot, cabin, chrome trim --- */
    c.fillStyle = bg;
    crRoundRect(c, cx - cw / 2, cy - ch * 0.72, cw, ch * 0.72, cw * 0.10);
    // Boot lid highlight.
    c.fillStyle = 'rgba(255,255,255,.10)';
    c.fillRect(cx - cw * 0.42, cy - ch * 0.70, cw * 0.84, ch * 0.06);
    // Cabin shoulders.
    c.fillStyle = col;
    c.beginPath();
    c.moveTo(cx - cw * 0.37, cy - ch * 0.70);
    c.quadraticCurveTo(cx, cy - ch * 1.04, cx + cw * 0.37, cy - ch * 0.70);
    c.closePath(); c.fill();
    // Rear window with reflection.
    c.fillStyle = 'rgba(9,13,22,.88)';
    crRoundRect(c, cx - cw * 0.29, cy - ch * 0.92, cw * 0.58, ch * 0.31, cw * 0.05);
    c.fillStyle = 'rgba(140,190,255,.14)';
    c.fillRect(cx - cw * 0.26, cy - ch * 0.90, cw * 0.52, ch * 0.055);
    // Chrome boot trim strip.
    c.fillStyle = 'rgba(200,215,235,.22)';
    c.fillRect(cx - cw * 0.40, cy - ch * 0.50, cw * 0.80, ch * 0.02);
    // Bumper + plate.
    c.fillStyle = 'rgba(10,13,20,.62)';
    c.fillRect(cx - cw * 0.44, cy - ch * 0.16, cw * 0.88, ch * 0.13);
    c.fillStyle = 'rgba(210,220,235,.72)';
    c.fillRect(cx - cw * 0.08, cy - ch * 0.15, cw * 0.16, ch * 0.075);
  }

  // --- tail lights: bigger, brighter, glowing when close --------------
  // Phase 5 fix: the old glow term (p.scale*3 ≈ 0.0004) never lit up.
  // Now it keys off drawn size, which is what the eye actually tracks.
  const glow = Math.min(1, cw / 26);
  const lw = cw * 0.15, lh = Math.max(1.4, ch * 0.11);
  const ly = cy - ch * (style === 'truck' || style === 'van' ? 0.30 : 0.38);
  if (glow > 0.35 && cw > 9) {
    c.shadowColor = 'rgba(255,70,95,.9)';
    c.shadowBlur = Math.min(14, cw * 0.35);
  }
  c.fillStyle = `rgba(255,80,105,${(0.65 + glow * 0.35).toFixed(3)})`;
  c.fillRect(cx - cw * 0.42, ly, lw, lh);
  c.fillRect(cx + cw * 0.42 - lw, ly, lw, lh);
  c.shadowBlur = 0;
  // Inner bright cores.
  c.fillStyle = `rgba(255,190,205,${(0.5 * glow).toFixed(3)})`;
  c.fillRect(cx - cw * 0.42 + lw * 0.2, ly + lh * 0.2, lw * 0.6, lh * 0.4);
  c.fillRect(cx + cw * 0.42 - lw * 1.2 + lw * 0.2, ly + lh * 0.2, lw * 0.6, lh * 0.4);

  // --- headlight spill onto the road toward the camera (near only) ----
  if (cw > 22) {
    const spill = c.createLinearGradient(0, cy, 0, cy + ch * 0.55);
    spill.addColorStop(0, 'rgba(255,240,215,.11)');
    spill.addColorStop(1, 'rgba(255,240,215,0)');
    c.fillStyle = spill;
    c.fillRect(cx - cw * 0.42, cy, cw * 0.84, ch * 0.55);
  }
  c.restore();
}

/** Traffic wheel: tyre + rim + hub, sized to the body. Detail appears
 *  only when large enough on screen to be worth the fills. */
function crDrawTrafficWheel(c, x, y, r) {
  c.fillStyle = '#07090e';
  c.beginPath(); c.arc(x, y, r, 0, Math.PI * 2); c.fill();
  if (r > 2.2) {
    c.fillStyle = '#232c3d';
    c.beginPath(); c.arc(x, y, r * 0.58, 0, Math.PI * 2); c.fill();
    if (r > 4.5) {
      c.fillStyle = '#8b96ad';
      c.beginPath(); c.arc(x, y, r * 0.20, 0, Math.PI * 2); c.fill();
      // Two lug spokes (cheap hint of a rim at distance).
      c.strokeStyle = 'rgba(139,150,173,.55)';
      c.lineWidth = Math.max(0.8, r * 0.13);
      c.beginPath();
      c.moveTo(x - r * 0.45, y); c.lineTo(x + r * 0.45, y);
      c.moveTo(x, y - r * 0.45); c.lineTo(x, y + r * 0.45);
      c.stroke();
    }
  }
}

function crRoundRect(c, x, y, w, h, r) {
  c.beginPath();
  c.moveTo(x + r, y);
  c.arcTo(x + w, y, x + w, y + h, r);
  c.arcTo(x + w, y + h, x, y + h, r);
  c.arcTo(x, y + h, x, y, r);
  c.arcTo(x, y, x + w, y, r);
  c.closePath(); c.fill();
}

/** Draw a spoked wheel. `spin` is accumulated radians so the wheel visibly
 *  rotates with road speed; at high speed the spokes blur into a disc. */
/** PHASE 6 · lighten (k>0) / darken (k<0) a #rrggbb colour. */
function crShade(hex, k) {
  const n = parseInt(hex.slice(1), 16);
  let r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255;
  if (k >= 0) { r += (255 - r) * k; g += (255 - g) * k; b += (255 - b) * k; }
  else        { r *= 1 + k; g *= 1 + k; b *= 1 + k; }
  return 'rgb(' + (r|0) + ',' + (g|0) + ',' + (b|0) + ')';
}

function crDrawWheel(c, x, y, r, spin) {
  /* PHASE 6: rim/spoke colours come from the selected wheel style. */
  const wDef = CR_WHEELS.find(w => w.id === CR_SAVE.custom.wheel) || CR_WHEELS[0];
  // Tyre.
  c.fillStyle = '#0a0d13';
  c.beginPath(); c.arc(x, y, r, 0, Math.PI * 2); c.fill();
  // Rim.
  c.fillStyle = wDef.rim;
  c.beginPath(); c.arc(x, y, r * 0.62, 0, Math.PI * 2); c.fill();
  // Spokes (skipped when tiny on screen).
  if (r > 5) {
    c.strokeStyle = wDef.spoke;
    c.lineWidth = Math.max(1, r * 0.14);
    for (let i = 0; i < 5; i++) {
      const a = spin + (i * Math.PI * 2) / 5;
      c.beginPath();
      c.moveTo(x, y);
      c.lineTo(x + Math.cos(a) * r * 0.55, y + Math.sin(a) * r * 0.55);
      c.stroke();
    }
  }
}

/** The player's car: a proper rear-view sports silhouette drawn in vector
 *  form — wide stance, low roofline, spoiler, tinted glass, exhausts.
 *  Everything scales from one width constant so it stays proportional.
 *
 *  Feedback baked in:
 *   • body roll follows lateral slip (weight transfer into a slide)
 *   • brake lights flare when braking
 *   • wheels rotate with road speed
 *   • suspension squat under acceleration / dive under braking
 *   • nitro flames only while actually burning */
function crDrawPlayer(c, W, H) {
  const g = CRG;
  const dtDraw = g._dtDraw || 0;
  /* ------------------------------------------------------------------
   * PREMIUM REAR-VIEW SPORTS CAR — built from layered vector shapes:
   *   shadow → tyres → diffuser → lower body → haunches → cabin glass →
   *   roof → spoiler → light bar → details.
   * Everything derives from one width constant (cw), so the car keeps its
   * proportions at any canvas scale. Feedback baked in:
   *   • body roll follows lateral slip (weight transfer into a slide)
   *   • brake lights flare when braking
   *   • wheels rotate with road speed
   *   • suspension squat under acceleration / dive under braking
   *   • nitro flames only while actually burning
   *   • subtle idle bounce so the car never looks frozen
   * ------------------------------------------------------------------ */
  /* PHASE 5 · sports proportions: wider track, lower glasshouse, longer
   * rear deck — a proper GT silhouette instead of a tall hatch shape. */
  /* PHASE 6 · garage cars: each body id gets its own silhouette tweak
   * (roof height, haunch width, deck length) plus the player's chosen
   * paint, neon, wheels, spoiler and plate from CR_SAVE.custom. */
  const cust = CR_SAVE.custom;
  const carDef = CR_CARS.find(c => c.id === (g.carId || CR_SAVE.car)) || CR_CARS[0];
  const BODY = {
    gt:      { w: 156, h: 74, roof: 1.00, haunch: 1.00 },
    wedge:   { w: 150, h: 66, roof: 0.90, haunch: 0.94 },
    drift:   { w: 158, h: 72, roof: 0.97, haunch: 1.06 },
    muscle:  { w: 168, h: 78, roof: 1.05, haunch: 1.12 },
    phantom: { w: 160, h: 70, roof: 0.93, haunch: 1.02 },
  }[carDef.body] || { w: 156, h: 74, roof: 1, haunch: 1 };
  /* TASK 7.0 · the player is drawn ON its own projection plane. With the
   * camera trailing by CAM_TRAIL, the player's world position (rel=0)
   * projects to a fixed screen row and scale — computed here from the SAME
   * crProject model the road and traffic use, so all three share one
   * perspective. The sprite width becomes the projected width of
   * PLAYER_WIDTH road units × the same CAR_VIS_K factor as traffic:
   * cars and player are now in identical units at identical depths. */
  /* PLAYER-TRACK FIX · the car is now positioned by TRUE PROJECTION.
   * The old code drew the sprite at W/2 + (playerX-camX)*W*0.09 — an
   * arbitrary screen-space offset that ignored the road's accumulated
   * curve offset, the perspective scale and the camera geometry. In
   * corners the road swept sideways while the car stayed put: up to a
   * 90px disconnect (measured), which read as the car sliding on top of
   * a separate background instead of driving ON the track.
   * Now: world lateral = curveX(playerDepth) + playerX*ROAD_W, projected
   * through the very same crProject() the tarmac uses — one coordinate
   * system for road, traffic, scenery and player. The camera lag (camX
   * easing toward the player) still produces the natural "car moves
   * within the frame" feel, because crProject subtracts camX internally
   * exactly as it does for the road. */
  const pCurveX = (CR._curveX && CR._curveX.length)
    ? CR._curveX[Math.min(CR.DRAW_DIST + 1,
        Math.max(0, Math.round(CR.CAM_TRAIL / CR.SEG_LEN)))]
    : 0;
  const pSelf = crProject(CR.CAM_TRAIL,
                          pCurveX + g.playerX * CR.ROAD_W, 0);
  /* Width uses the SAME chain as traffic (p.w * 0.56 * bodyK * CAR_VIS_K)
   * with the player's own body factor — identical units, identical depth,
   * so a sedan alongside renders at a believable size relative to us. */
  /* bodyK: the player's road-unit width, derived from its garage body
   * proportions (gt≈0.60 units, muscle wider, wedge narrower) — directly
   * comparable to traffic car.w values. */
  const bodyK = 0.72 * (BODY.w / 156);
  const cw = Math.max(90, pSelf.w * 0.56 * bodyK * CR.CAR_VIS_K);
  const ch = cw * (BODY.h / BODY.w);
  // Screen x IS the projection — the car sits exactly where its world
  // position lands on the projected road. (No arbitrary offsets.)
  const cx = pSelf.x;
  const speedRatio = g.speed / g.maxSpeed;

  /* PHASE 5 · suspension. Two independent springs:
   *   • heave (vertical): eased squat/dive from longitudinal load —
   *     braking dives the nose, acceleration squats the tail.
   *   • roll is already handled by g.roll (slip-driven).
   * The spring rate is frame-rate independent (exp ease), so the motion
   * looks identical at 60 and 144 Hz. */
  g.squat = g.squat === undefined ? 0 : g.squat;
  const accelNow = (g.nitroActive ? 1 : 0.55) * (g.braking ? -1 : 1);
  const squatTarget = g.braking ? -3.6 : speedRatio * 2.6 + accelNow * 0.8;
  g.squat += (squatTarget - g.squat) * (1 - Math.exp(-dtDraw * 9));
  const vib = (g.speed > 200 && !g.reducedMotion)
    ? Math.sin(performance.now() / 34) * (0.35 + speedRatio * 0.75) : 0;
  /* Ground the player on its projected row: the road surface at rel=0
   * (≈H-6 at rest). The clamp keeps suspension bob from sliding the
   * sprite off the canvas edge. */
  const cy = Math.min(H - 8, pSelf.y) + g.squat + vib;

  c.save();
  c.translate(cx, cy);
  if (g.roll) c.rotate(g.roll);

  const hw = cw / 2;

  /* PHASE 5 · layered ground shadow: a wide soft penumbra (the car's
   * mass), a tighter core (direct occlusion) and two contact patches
   * under the tyres — the last is what actually anchors a car to the
   * road visually. All scale slightly with speed for weight feel. */
  const shA = 0.28 + speedRatio * 0.18;
  c.fillStyle = `rgba(0,0,0,${shA.toFixed(3)})`;
  c.beginPath();
  c.ellipse(0, 7, cw * (0.58 + speedRatio * 0.04), 19, 0, 0, Math.PI * 2);
  c.fill();
  c.fillStyle = `rgba(0,0,0,${(shA + 0.10).toFixed(3)})`;
  c.beginPath();
  c.ellipse(0, 4, cw * 0.47, 12, 0, 0, Math.PI * 2);
  c.fill();
  // Contact patches (darker ellipse under each rear tyre).
  for (const ox of [-hw + 13, hw - 13]) {
    c.fillStyle = `rgba(0,0,0,${(shA + 0.22).toFixed(3)})`;
    c.beginPath();
    c.ellipse(ox, 2, 15, 5.5, 0, 0, Math.PI * 2);
    c.fill();
  }
  /* PHASE 6 · neon underglow: a soft additive ellipse in the chosen
   * accent colour under the body — the customization's most visible
   * gameplay cue. Intensity swells slightly with nitro. */
  {
    const glowA = 0.16 + (g.nitroVis || 0) * 0.20;
    const gg = c.createRadialGradient(0, 4, cw * 0.18, 0, 4, cw * 0.62);
    gg.addColorStop(0, CR_SAVE.custom.neon);
    gg.addColorStop(1, 'rgba(0,0,0,0)');
    c.save();
    c.globalCompositeOperation = 'lighter';
    c.globalAlpha = glowA;
    c.fillStyle = gg;
    c.beginPath(); c.ellipse(0, 4, cw * 0.60, 20, 0, 0, Math.PI * 2); c.fill();
    c.restore();
  }

  // --- rear wheels (drawn first, partly behind the body) ---------------
  // Slightly wider track than the body so the tyres read at the fender lips.
  const wr = 14.5;                                // wheel radius
  const wy = -wr - 1;
  const woff = hw - 13;
  crDrawWheel(c, -woff, wy, wr, g.wheelSpin);
  crDrawWheel(c, woff, wy, wr, g.wheelSpin);

  // --- diffuser (dark carbon base with vertical fins) ------------------
  c.fillStyle = '#0a0e15';
  c.fillRect(-hw + 10, -12, cw - 20, 14);
  c.fillStyle = '#151d2b';
  for (let i = 0; i < 6; i++) {
    const fx = -hw + 22 + i * ((cw - 44) / 5);
    c.fillRect(fx - 1.6, -11, 3.2, 12);
  }

  // --- main body: low, wide, wedge-shaped ------------------------------
  // Layered path: bumper -> haunches -> roof, each lighter than the last,
  // which reads as glossy paint catching the neon above.
  // Custom paint: lighten/darken the chosen hex for a 4-stop gradient.
  const paint = crShade(cust.body, 0.55),         // roof highlight
        paint2 = crShade(cust.body, 0.0),          // mid
        paint3 = crShade(cust.body, -0.28),        // flank
        paint4 = crShade(cust.body, -0.52);        // lower shadow
  let grad = c.createLinearGradient(0, -ch, 0, 4);
  grad.addColorStop(0, paint);
  grad.addColorStop(0.30, paint2);
  grad.addColorStop(0.62, paint3);
  grad.addColorStop(1, paint4);
  c.fillStyle = grad;
  c.beginPath();
  // Wide rear haunches (muscular hips), tapering toward the roof-line.
  c.moveTo(-hw, -8);
  c.lineTo(-hw + 3, -ch * 0.54);                  // left flank rises
  c.quadraticCurveTo(-hw + 9, -ch * 0.68, -hw + 27, -ch * 0.70);
  c.lineTo(hw - 27, -ch * 0.70);
  c.quadraticCurveTo(hw - 9, -ch * 0.68, hw - 3, -ch * 0.54);
  c.lineTo(hw, -8);
  c.quadraticCurveTo(hw, 2, hw - 13, 3);
  c.lineTo(-hw + 13, 3);
  c.quadraticCurveTo(-hw, 2, -hw, -8);
  c.closePath(); c.fill();

  // Paint sheen: a bright horizontal band across the upper body — the
  // "wet look" reflection of the sky/city on the boot lid.
  let sheen = c.createLinearGradient(0, -ch * 0.70, 0, -ch * 0.34);
  sheen.addColorStop(0, 'rgba(255,255,255,.30)');
  sheen.addColorStop(0.45, 'rgba(255,255,255,.10)');
  sheen.addColorStop(1, 'rgba(255,255,255,0)');
  c.fillStyle = sheen;
  c.fillRect(-hw + 6, -ch * 0.70, cw - 12, ch * 0.36);

  // Fender highlight arcs over each rear wheel.
  c.strokeStyle = 'rgba(190,255,235,.38)';
  c.lineWidth = 2;
  c.beginPath(); c.arc(-woff, -ch * 0.16, 25, Math.PI * 1.08, Math.PI * 1.92); c.stroke();
  c.beginPath(); c.arc(woff, -ch * 0.16, 25, Math.PI * 1.08, Math.PI * 1.92); c.stroke();

  // --- cabin: roof + tinted rear window --------------------------------
  c.fillStyle = 'rgba(8,11,19,.94)';
  c.beginPath();
  c.moveTo(-hw + 31, -ch * 0.68);
  c.quadraticCurveTo(-hw + 35, -ch * 1.00, -hw + 50, -ch * 1.02);
  c.lineTo(hw - 50, -ch * 1.02);
  c.quadraticCurveTo(hw - 35, -ch * 1.00, hw - 31, -ch * 0.68);
  c.closePath(); c.fill();
  // Rear windshield: angled glass with a cyan city reflection streak.
  let glass = c.createLinearGradient(0, -ch, 0, -ch * 0.68);
  glass.addColorStop(0, 'rgba(120,200,255,.30)');
  glass.addColorStop(0.5, 'rgba(60,90,140,.18)');
  glass.addColorStop(1, 'rgba(10,14,24,.05)');
  c.fillStyle = glass;
  c.beginPath();
  c.moveTo(-hw + 36, -ch * 0.70);
  c.quadraticCurveTo(-hw + 39, -ch * 0.97, -hw + 52, -ch * 0.985);
  c.lineTo(hw - 52, -ch * 0.985);
  c.quadraticCurveTo(hw - 39, -ch * 0.97, hw - 36, -ch * 0.70);
  c.closePath(); c.fill();
  // Glass highlight strip (sharp specular).
  c.fillStyle = 'rgba(160,215,255,.28)';
  c.fillRect(-hw + 42, -ch * 0.99, cw - 84, 2.5);

  /* --- rear wing / spoiler (PHASE 6: selectable styles) ----------------
   * duck     small lip on the deck
   * gt       full wing with endplates (default)
   * none     clean deck
   * twin     twin-strut wing, lower and wider                       */
  const spId = CR_SAVE.custom.spoiler;
  const neonHex = CR_SAVE.custom.neon;
  if (spId === 'duck') {
    c.fillStyle = crShade(cust.body, -0.45);
    c.fillRect(-hw + 16, -ch * 0.80, cw - 32, 6);
    c.fillStyle = neonHex + '';
    c.globalAlpha = 0.35; c.fillRect(-hw + 16, -ch * 0.80, cw - 32, 1.6);
    c.globalAlpha = 1;
  } else if (spId === 'twin') {
    c.fillStyle = '#10161f';
    c.fillRect(-hw + 26, -ch * 0.98, 6, ch * 0.30);   // struts
    c.fillRect(hw - 32, -ch * 0.98, 6, ch * 0.30);
    c.fillStyle = '#0e141c';
    c.fillRect(-hw + 10, -ch * 1.04, cw - 20, 6);     // blade
    c.globalAlpha = 0.30; c.fillStyle = neonHex;
    c.fillRect(-hw + 10, -ch * 1.04, cw - 20, 1.6); c.globalAlpha = 1;
  } else if (spId !== 'none') {                        // 'gt' default
    c.fillStyle = '#0e1d18';
    c.fillRect(-hw + 7, -ch * 1.10, cw - 14, 5);
    c.fillStyle = '#16352a';
    c.fillRect(-hw + 7, -ch * 1.12, 4, 9);          // left endplate
    c.fillRect(hw - 11, -ch * 1.12, 4, 9);          // right endplate
    c.fillStyle = '#12261f';
    c.fillRect(-hw + 18, -ch * 1.05, 5, 9);         // stands
    c.fillRect(hw - 23, -ch * 1.05, 5, 9);
    c.globalAlpha = 0.25; c.fillStyle = neonHex;
    c.fillRect(-hw + 7, -ch * 1.10, cw - 14, 1.4); c.globalAlpha = 1;
  }

  /* PHASE 5 · detail pass: side skirts, wheel-arch flares and a third
   * brake light. Small shapes, but they are what make the rear read as a
   * car body rather than a coloured slab. */
  // Side skirts (dark sills under each flank).
  c.fillStyle = 'rgba(7,10,16,.85)';
  c.fillRect(-hw + 2, -6, 13, 9);
  c.fillRect(hw - 15, -6, 13, 9);
  // Arch flares: a subtle lighter arc over each rear wheel.
  c.strokeStyle = 'rgba(190,255,235,.30)';
  c.lineWidth = 2.4;
  c.beginPath(); c.arc(-hw + 13, -ch * 0.14, 20, Math.PI * 1.05, Math.PI * 1.95); c.stroke();
  c.beginPath(); c.arc(hw - 13, -ch * 0.14, 20, Math.PI * 1.05, Math.PI * 1.95); c.stroke();
  // Third brake light: the roofline strip that flares when braking.
  c.fillStyle = g.braking ? '#ff2d55' : 'rgba(255,107,129,.45)';
  if (g.braking) { c.shadowColor = '#ff2d55'; c.shadowBlur = 12; }
  c.fillRect(-hw + 52, -ch * 1.045, cw - 104, 3);
  c.shadowBlur = 0;

  // --- racing stripe ----------------------------------------------------
  c.fillStyle = 'rgba(6,8,12,.5)';
  c.fillRect(-8, -ch * 0.70, 6, ch * 0.70);
  c.fillRect(2, -ch * 0.70, 6, ch * 0.70);

  // --- tail lights: wrap-around bars that FLARE when braking -----------
  const K = g.keys, T = g.touch;
  const braking = g.braking;
  c.fillStyle = braking ? '#ff2d55' : '#ff6b81';
  if (braking || g.nitroActive) {
    c.shadowColor = braking ? '#ff2d55' : '#5ef2c0';
    c.shadowBlur = 16;
  }
  // Two wrap-around light bars with an inner bright core.
  for (const ox of [-hw + 20, hw - 50]) {
    c.fillStyle = braking ? 'rgba(255,45,85,.95)' : 'rgba(255,107,129,.85)';
    c.fillRect(ox, -ch * 0.44, 30, 8);
    c.fillStyle = braking ? '#ffd0d9' : 'rgba(255,190,200,.75)';
    c.fillRect(ox + 3, -ch * 0.43, 24, 3);
  }
  c.shadowBlur = 0;
  // Centre light strip (modern EV-style bar).
  c.fillStyle = braking ? 'rgba(255,45,85,.9)' : 'rgba(255,107,129,.55)';
  c.fillRect(-hw + 54, -ch * 0.42, cw - 108, 4);

  // Number plate.
  c.fillStyle = 'rgba(210,220,235,.85)';
  c.fillRect(-16, -ch * 0.30, 32, 10);
  c.fillStyle = 'rgba(20,26,38,.9)';
  c.font = '700 7px ui-monospace, monospace';
  c.textAlign = 'center'; c.textBaseline = 'middle';
  c.fillText((CR_SAVE.custom.plate || 'SYN-01').slice(0, 8), 0, -ch * 0.30 + 5);

  // --- exhausts + nitro flames -----------------------------------------
  c.fillStyle = '#48566f';
  c.beginPath(); c.arc(-hw + 30, -3, 5, 0, Math.PI * 2); c.fill();
  c.beginPath(); c.arc(hw - 30, -3, 5, 0, Math.PI * 2); c.fill();
  c.fillStyle = '#1c2432';
  c.beginPath(); c.arc(-hw + 30, -3, 2.6, 0, Math.PI * 2); c.fill();
  c.beginPath(); c.arc(hw - 30, -3, 2.6, 0, Math.PI * 2); c.fill();
  if (g.nitroActive) {
    const fl = 34 + Math.random() * 30;
    const fg = c.createLinearGradient(0, 0, 0, fl);
    fg.addColorStop(0, 'rgba(220,190,255,.98)');
    fg.addColorStop(0.45, 'rgba(150,170,255,.75)');
    fg.addColorStop(1, 'rgba(111,183,255,0)');
    c.fillStyle = fg;
    for (const ox of [-hw + 30, hw - 30]) {
      c.beginPath();
      c.moveTo(ox - 6, 0); c.lineTo(ox, fl); c.lineTo(ox + 6, 0);
      c.closePath(); c.fill();
    }
    // Phase 3: cyan/blue afterburner trail particles peel off each exhaust
    // and streak down-screen behind the car. Anchored to the car's local
    // space here; stepped/drawn by the FX engine in screen space.
    if (!g.reducedMotion) {
      for (const ox of [-hw + 30, hw - 30]) {
        crFxSpawn('exhaust', {
          x: cx + ox, y: cy + 4,
          vx: (Math.random() - 0.5) * 26,
          vy: 130 + Math.random() * 120,
          ax: 0, ay: 260,
          r: 2.2 + Math.random() * 2.4,
          grow: 9,
          life: 1, decay: 2.6 + Math.random() * 0.8,
          hue: Math.random() < 0.72 ? 'c' : 'b',   // cyan or deep blue
        });
      }
    }
  }

  c.restore();
}

/** Draw + step the nitro exhaust trail (screen-space, additive). */
function crDrawExhaust(c, dt) {
  const arr = CR_FX.exhaust;
  if (!arr.length) return;
  crFxStep(arr, dt, 0.99, 0);
  c.save();
  c.globalCompositeOperation = 'lighter';
  for (const p of arr) {
    const k = Math.max(0, Math.min(1, p.life));
    // Cyan core with a blue-white flash when fresh.
    c.fillStyle = p.hue === 'c'
      ? `rgba(96,236,255,${(k * 0.55).toFixed(3)})`
      : `rgba(140,160,255,${(k * 0.45).toFixed(3)})`;
    c.beginPath(); c.arc(p.x, p.y, p.r * (0.5 + k * 0.8), 0, Math.PI * 2); c.fill();
    if (k > 0.7) {                                  // hot white core
      c.fillStyle = `rgba(230,245,255,${((k - 0.7) * 1.4).toFixed(3)})`;
      c.beginPath(); c.arc(p.x, p.y, p.r * 0.45, 0, Math.PI * 2); c.fill();
    }
  }
  c.restore();
}


/* =====================================================================
   PHASE 4 · INSTRUMENT CLUSTER (canvas-drawn, bottom corners)
   Left cluster : speed dial + km/h + nitro activation tick.
   Right cluster: RPM tacho arc + gear numeral + nitro energy arc.
   Everything derives from live state (g.speed / g.rpm / g.gear / g.nitro).
   Drawn AFTER the world but BEFORE popups; skipped on the ready overlay
   state so the intro screen stays clean. All strokes are simple arcs —
   no shadows, no gradients per frame beyond two cached ones.
   ===================================================================== */
const CR_KMH = 0.01;                      // world u/s -> display km/h
function crDrawCluster(c, W, H) {
  const g = CRG;
  if (g.state === 'ready') return;        // clean intro screen
  const rm = g.reducedMotion;

  /* ---------- LEFT: speed dial ---------- */
  /* TASK 7.0 · safe margins: screen shake translates the whole frame by
   * up to ~9px, so corner instruments are inset far enough that their
   * discs never cross the canvas edge at maximum shake. */
  const SAFE = 12;
  const R = Math.min(64, W * 0.085);      // dial radius, responsive
  const cx = SAFE + 10 + R, cy = H - SAFE - 10 - R;
  const kmh = g.speed * CR_KMH;
  const sr = Math.min(1, g.speed / g.maxSpeed);
  // Dial sweeps from 210° to -30° (a 240° gauge).
  const A0 = Math.PI * (7 / 6), SWEEP = Math.PI * (4 / 3);

  c.save();
  // Panel: dark translucent disc with a hairline ring.
  c.fillStyle = 'rgba(9,12,19,.72)';
  c.beginPath(); c.arc(cx, cy, R + 10, 0, Math.PI * 2); c.fill();
  c.strokeStyle = 'rgba(42,51,71,.9)'; c.lineWidth = 1;
  c.stroke();

  // Tick marks every 20 km/h-ish (9 major ticks across the sweep).
  for (let i = 0; i <= 8; i++) {
    const a = A0 + (SWEEP * i) / 8;
    const inner = i % 2 === 0 ? R - 12 : R - 8;
    c.strokeStyle = i >= 6 ? 'rgba(255,107,129,.55)' : 'rgba(154,165,184,.5)';
    c.lineWidth = i % 2 === 0 ? 2 : 1;
    c.beginPath();
    c.moveTo(cx + Math.cos(a) * inner, cy + Math.sin(a) * inner);
    c.lineTo(cx + Math.cos(a) * (R - 3), cy + Math.sin(a) * (R - 3));
    c.stroke();
  }

  // Speed arc (cyan → violet with speed; magenta while boosting).
  const arcA = A0 + SWEEP * sr;
  c.strokeStyle = g.nitroVis > 0.05 ? '#ff5ca8' : '#6fb7ff';
  c.lineWidth = 5; c.lineCap = 'round';
  c.beginPath(); c.arc(cx, cy, R - 16, A0, arcA); c.stroke();
  // Dim remainder track.
  c.strokeStyle = 'rgba(42,51,71,.55)'; c.lineWidth = 2;
  c.beginPath(); c.arc(cx, cy, R - 16, arcA, A0 + SWEEP); c.stroke();

  // Big digital number (animated by the same eased rpm smoothing? No —
  // speed itself is already continuous; tabular font stops jitter).
  c.fillStyle = sr > 0.92 ? '#ffc857' : '#e8ecf4';
  c.font = `800 ${Math.round(R * 0.52)}px ui-monospace, monospace`;
  c.textAlign = 'center'; c.textBaseline = 'alphabetic';
  c.fillText(String(Math.round(kmh)), cx, cy + R * 0.28);
  c.font = `700 ${Math.round(R * 0.17)}px ui-monospace, monospace`;
  c.fillStyle = 'rgba(154,165,184,.9)';
  c.fillText('KM/H', cx, cy + R * 0.58);

  // Nitro activation flag under the dial.
  if (g.nitroActive || g.nitroVis > 0.05) {
    const na = Math.min(1, g.nitroVis);
    c.fillStyle = `rgba(255,92,168,${(0.55 + 0.45 * na).toFixed(3)})`;
    c.font = `800 ${Math.round(R * 0.19)}px ui-monospace, monospace`;
    c.fillText('N₂O', cx, cy + R * 0.86);
  }
  c.restore();

  /* ---------- RIGHT: RPM + gear + nitro arc ---------- */
  const R2 = Math.min(56, W * 0.075);
  const gx = W - SAFE - 10 - R2, gy = H - SAFE - 10 - R2;
  c.save();
  c.fillStyle = 'rgba(9,12,19,.72)';
  c.beginPath(); c.arc(gx, gy, R2 + 10, 0, Math.PI * 2); c.fill();
  c.strokeStyle = 'rgba(42,51,71,.9)'; c.lineWidth = 1;
  c.stroke();

  // RPM arc: 180° top semicircle, redline zone marked in red.
  const RA0 = Math.PI, RS = Math.PI;
  // Redline zone (last 18% of rev range).
  c.strokeStyle = 'rgba(255,107,129,.75)'; c.lineWidth = 4;
  c.beginPath(); c.arc(gx, gy, R2 - 12, RA0 + RS * 0.82, RA0 + RS); c.stroke();
  // Active revs (violet; red past the redline gate).
  const revA = RA0 + RS * g.rpm;
  c.strokeStyle = g.rpm > 0.82 ? '#ff6b81' : '#b48cff';
  c.lineWidth = 5; c.lineCap = 'round';
  c.beginPath(); c.arc(gx, gy, R2 - 12, RA0, revA); c.stroke();
  // Shift lamp: solid red dot when in the redline with throttle.
  if (g.rpm > 0.82) {
    const blink = rm ? 1 : (Math.sin(performance.now() / 90) * 0.5 + 0.5);
    c.fillStyle = `rgba(255,80,100,${(0.35 + 0.65 * blink).toFixed(3)})`;
    c.beginPath(); c.arc(gx, gy - R2 + 2, 4, 0, Math.PI * 2); c.fill();
  }

  // Gear numeral (large, centre).
  c.fillStyle = '#e8ecf4';
  c.font = `800 ${Math.round(R2 * 0.62)}px ui-monospace, monospace`;
  c.textAlign = 'center'; c.textBaseline = 'middle';
  c.fillText(String(g.gear), gx, gy + R2 * 0.10);
  c.font = `700 ${Math.round(R2 * 0.16)}px ui-monospace, monospace`;
  c.fillStyle = 'rgba(154,165,184,.9)';
  c.fillText('GEAR', gx, gy + R2 * 0.48);

  // Nitro energy arc hugs the bottom of the right dial.
  const NA0 = Math.PI * 0.15, NS = Math.PI * 0.7;
  const nA = NA0 + NS * g.nitro;
  // Track.
  c.strokeStyle = 'rgba(42,51,71,.55)'; c.lineWidth = 3;
  c.beginPath(); c.arc(gx, gy, R2 + 4, NA0, NA0 + NS); c.stroke();
  // Fill: cyan normally, amber when low, glow-cyan at full.
  const low = g.nitro < 0.25;
  c.strokeStyle = low ? '#ffc857' : (g.nitro >= 0.999 ? '#5ef2c0' : '#40e0ff');
  c.lineWidth = low ? 4 : 3; c.lineCap = 'round';
  c.beginPath(); c.arc(gx, gy, R2 + 4, NA0, nA); c.stroke();
  // Full-charge pulse ring.
  if (g.nitro >= 0.999 && !rm) {
    const pu = (Math.sin(performance.now() / 260) * 0.5 + 0.5);
    c.strokeStyle = `rgba(94,242,192,${(pu * 0.4).toFixed(3)})`;
    c.lineWidth = 2;
    c.beginPath(); c.arc(gx, gy, R2 + 8 + pu * 3, NA0, NA0 + NS); c.stroke();
  }
  c.restore();
}

/* =====================================================================
   PHASE 4 · OPPONENT RADAR (top-centre canvas strip)
   A compact horizontal band showing every traffic car within ~40 segments:
     • horizontal axis = lateral lane position (real car.x, -1..1)
     • vertical axis   = distance ahead of the player (0..40 segments)
     • player marker   = cyan chevron at the bottom centre
   Indicators ease toward their true slot (smooth animation) but the DATA
   is exact: one pass over g.traffic per frame, capped at the nearest 8.
   ===================================================================== */
const CR_RADAR = [];                       // smoothed indicator slots
function crDrawRadar(c, W, H) {
  const g = CRG;
  if (g.state === 'ready') return;
  const rw = Math.min(190, W * 0.24), rh = 64;
  const rx = W / 2 - rw / 2, ry = 12;   // TASK 7.0: shake-safe top margin

  c.save();
  // Panel.
  c.fillStyle = 'rgba(9,12,19,.66)';
  crRoundRect(c, rx, ry, rw, rh, 10);
  c.strokeStyle = 'rgba(42,51,71,.8)'; c.lineWidth = 1;
  c.stroke();
  // Lane grid: two faint dividers for the three lanes.
  c.strokeStyle = 'rgba(42,51,71,.5)';
  for (const t of [-1 / 3, 1 / 3]) {
    const lx = rx + rw * ((t + 1) / 2);
    c.beginPath(); c.moveTo(lx, ry + 6); c.lineTo(lx, ry + rh - 12); c.stroke();
  }
  // Horizon line = far edge of the radar window.
  c.strokeStyle = 'rgba(90,101,126,.45)';
  c.beginPath(); c.moveTo(rx + 6, ry + 6); c.lineTo(rx + rw - 6, ry + 6); c.stroke();

  // Player chevron at the bottom.
  const pxs = rx + rw * ((Math.max(-1, Math.min(1, g.playerX)) + 1) / 2);
  c.fillStyle = '#5ef2c0';
  c.beginPath();
  c.moveTo(pxs, ry + rh - 6);
  c.lineTo(pxs - 5, ry + rh - 13);
  c.lineTo(pxs + 5, ry + rh - 13);
  c.closePath(); c.fill();

  // Collect the nearest cars ahead (sorted by rel, capped at 8).
  const playerSeg = g.position / CR.SEG_LEN;
  const near = [];
  for (const car of g.traffic) {
    let rel = car.z - playerSeg;
    if (rel < -g.totalSegs / 2) rel += g.totalSegs;
    if (rel > g.totalSegs / 2) rel -= g.totalSegs;
    if (rel >= -1 && rel <= 40) near.push({ rel, x: car.x });
  }
  near.sort((a, b) => a.rel - b.rel);
  if (near.length > 8) near.length = 8;

  /* Smoothed slots: one per visible car, reused by index. Surplus slots
   * fade out and are then COMPACTED OUT of the array — a cleared road must
   * leave zero slots, not a lingering residue (write-index compaction, no
   * allocation churn). */
  while (CR_RADAR.length < near.length) CR_RADAR.push({ x: 0, y: 0, a: 0 });
  let rw2 = 0;
  for (let i = 0; i < CR_RADAR.length; i++) {
    const s = CR_RADAR[i];
    if (i < near.length) {
      const tx = rx + rw * ((Math.max(-1, Math.min(1, near[i].x)) + 1) / 2);
      const ty = ry + 8 + (rh - 24) * (1 - Math.max(0, near[i].rel) / 40);
      // Ease toward the true slot (fast when appearing, gentle while held).
      s.x += (tx - s.x) * 0.25;
      s.y += (ty - s.y) * 0.25;
      s.a = Math.min(1, s.a + 0.2);
    } else {
      s.a -= 0.15;                        // fading out
    }
    if (s.a <= 0) continue;               // dropped: not kept, not drawn
    // Indicator: amber dot, red-tinged when very close ahead.
    const closing = i < near.length && near[i].rel < 4;
    c.fillStyle = closing
      ? `rgba(255,140,87,${(s.a * 0.95).toFixed(3)})`
      : `rgba(180,140,255,${(s.a * 0.85).toFixed(3)})`;
    c.beginPath(); c.arc(s.x, s.y, closing ? 3.4 : 2.6, 0, Math.PI * 2); c.fill();
    CR_RADAR[rw2++] = s;
  }
  CR_RADAR.length = rw2;
  c.restore();
}

/* =====================================================================
   BUGFIX 10 · RACE MINIMAP
   A real closed-circuit outline derived from the ACTUAL track geometry:
   heading is integrated over the same per-segment `curve` values the
   world uses, so the shape on the minimap IS the shape you drive. Built
   once per track (cached), drawn small at the top-right. Player = cyan
   dot, AI opponents = magenta dots, at their true segment positions.
   Lap modes only (it is a race map, not a traffic map).
   ===================================================================== */
const CR_MINIMAP = { pts: null, bounds: null };

/** Integrate the track outline into normalised 2D points (0..1 each axis).
 *  Deterministic: same track -> same shape. */
function crMinimapBuild() {
  const g = CRG;
  const n = g.totalSegs;
  /* P2.8 · the heading factor is derived from the track itself so the
   * integrated loop CLOSES: a real circuit turns exactly ±360° over one
   * lap. The old fixed 0.0175 rad/unit left this track at -27° net turn,
   * so the "closed" outline was really an open spiral force-wrapped shut.
   * Normalising the total turn to a full circle keeps every bend's RELATIVE
   * shape (same proportions, same corner order) while guaranteeing closure
   * for any track layout. */
  let sumCurve = 0;
  for (let i = 0; i < n; i++) sumCurve += g.segs[i].curve;
  const TURN = 2 * Math.PI;                       // one full revolution
  const k = Math.abs(sumCurve) > 1e-6 ? TURN / Math.abs(sumCurve) : 0;
  const dir = sumCurve >= 0 ? 1 : -1;             // dominant turn direction
  const xs = new Float64Array(n + 1), ys = new Float64Array(n + 1);
  let x = 0, y = 0, heading = 0;
  for (let i = 0; i < n; i++) {
    heading += g.segs[i].curve * k * dir;
    x += Math.cos(heading);
    y += Math.sin(heading);
    xs[i] = x; ys[i] = y;
  }
  xs[n] = x; ys[n] = y;
  /* P2.8 · position closure. A full 360° of heading does not by itself
   * return the pen to the origin (turn is not uniformly distributed —
   * real circuits have straights). Distribute the residual endpoint error
   * along the polyline with a smooth sin² weight (zero at the start, one
   * at the end): the outline bends gently to close instead of force-
   * wrapping the last point, so every bend keeps its local shape. */
  {
    const ex = xs[n] - xs[0], ey = ys[n] - ys[0];
    if (Math.abs(ex) > 1e-9 || Math.abs(ey) > 1e-9) {
      for (let i = 0; i <= n; i++) {
        const t = i / n, w = Math.sin(t * Math.PI / 2);
        xs[i] -= ex * w * w;
        ys[i] -= ey * w * w;
      }
    }
  }
  // Normalise into 0..1 (aspect preserved).
  let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
  for (let i = 0; i < n; i++) {
    if (xs[i] < minX) minX = xs[i]; if (xs[i] > maxX) maxX = xs[i];
    if (ys[i] < minY) minY = ys[i]; if (ys[i] > maxY) maxY = ys[i];
  }
  const spanX = Math.max(1e-6, maxX - minX), spanY = Math.max(1e-6, maxY - minY);
  const span = Math.max(spanX, spanY);
  const pts = [];
  for (let i = 0; i <= n; i++) {
    const j = i % n;
    pts.push({ x: (xs[j] - minX) / span, y: (ys[j] - minY) / span });
  }
  CR_MINIMAP.pts = pts;
  CR_MINIMAP.span = span;
  CR_MINIMAP.startIdx = 0;
  return pts;
}

/** Draw the minimap (top-right corner, lap modes only). */
function crDrawMinimap(c, W, H) {
  const g = CRG, L = CR_LAP;
  if (!L.enabled || g.state === 'ready') return;
  if (!CR_MINIMAP.pts) crMinimapBuild();
  const pts = CR_MINIMAP.pts;
  const mw = Math.min(118, W * 0.16), mh = mw;     // square map
  const mx = W - mw - 14, my = 14;                 // top-right, shake-safe
  c.save();
  // Panel.
  c.fillStyle = 'rgba(9,12,19,.66)';
  crRoundRect(c, mx - 6, my - 6, mw + 12, mh + 12, 8);
  c.strokeStyle = 'rgba(42,51,71,.8)'; c.lineWidth = 1;
  c.stroke();
  // Track outline: closed loop through the integrated points.
  c.strokeStyle = 'rgba(64,224,255,.45)';
  c.lineWidth = 2; c.lineJoin = 'round';
  c.beginPath();
  for (let i = 0; i < pts.length; i += 3) {        // stride: shape, not cost
    const px = mx + pts[i].x * mw, py = my + pts[i].y * mh;
    if (i === 0) c.moveTo(px, py); else c.lineTo(px, py);
  }
  c.closePath();
  c.stroke();
  // Start/finish tick (segment 0 direction marker).
  const p0 = pts[0], p1 = pts[Math.min(6, pts.length - 1)];
  const ang = Math.atan2(p1.y - p0.y, p1.x - p0.x);
  c.strokeStyle = '#ffc857'; c.lineWidth = 2;
  c.beginPath();
  c.moveTo(mx + p0.x * mw - Math.cos(ang) * 4, my + p0.y * mh - Math.sin(ang) * 4);
  c.lineTo(mx + p0.x * mw + Math.cos(ang) * 4, my + p0.y * mh + Math.sin(ang) * 4);
  c.stroke();
  // Map a continuous segment index (across laps) to a point on the loop.
  const mapPt = (segF) => {
    const s = ((segF % g.totalSegs) + g.totalSegs) % g.totalSegs;
    const f = s / g.totalSegs * (pts.length - 1);
    const i = Math.floor(f), k = f - i;
    const A = pts[Math.min(i, pts.length - 1)], B = pts[Math.min(i + 1, pts.length - 1)];
    return { x: mx + (A.x + (B.x - A.x) * k) * mw,
             y: my + (A.y + (B.y - A.y) * k) * mh };
  };
  // AI opponents (magenta, small).
  for (const r of CR_RACERS) {
    if (r.isPlayer || !r.ai) continue;
    const p = mapPt(r.ai.segF);
    c.fillStyle = '#ff5ca8';
    c.beginPath(); c.arc(p.x, p.y, 2.4, 0, Math.PI * 2); c.fill();
  }
  // Player (cyan, larger, glowing).
  const me = CR_RACERS.find(r => r.isPlayer);
  if (me) {
    const p = mapPt(g.position / CR.SEG_LEN);
    c.shadowColor = '#40e0ff'; c.shadowBlur = 6;
    c.fillStyle = '#40e0ff';
    c.beginPath(); c.arc(p.x, p.y, 3.4, 0, Math.PI * 2); c.fill();
    c.shadowBlur = 0;
  }
  c.restore();
}

/* TASK 7.2 · racing position indicator: "P1 /4" with lap and the gap to
 * the racer ahead/behind, drawn top-LEFT (the radar owns top-centre) so
 * it never covers the road or the player car. Smooth transitions: the
 * big numeral eases its scale on change and gaps are sampled from live
 * progress. Only renders in lap modes with a registered field. */
const CR_POSHUD = { flashT: 0, lastPos: 1 };
function crDrawPositionHud(c, W, H, dtDraw) {
  const g = CRG, L = CR_LAP;
  if (!L.enabled || !CR_RACERS.length) return;
  const me = CR_RACERS.find(r => r.isPlayer);
  if (!me) return;

  // Scale pulse on a position CHANGE (both directions — gaining and
  // losing a place are equally worth noticing).
  if (me.pos !== CR_POSHUD.lastPos) { CR_POSHUD.flashT = 0.45; CR_POSHUD.lastPos = me.pos; }
  if (CR_POSHUD.flashT > 0) CR_POSHUD.flashT = Math.max(0, CR_POSHUD.flashT - dtDraw);

  /* BUGFIX 11 · left race panel was clipped at the screen edge when
   * screen shake translated the frame: margin raised from 12 to 16 so
   * even maximum shake (+9px) keeps the full panel on-canvas at any
   * viewport size. Width also clamped against the actual canvas width. */
  const px = 16, py = 16;              // shake-safe margin (max shake ~9px)
  const pw = Math.max(120, Math.min(178, W * 0.24)), ph = 78;

  c.save();
  // Panel.
  c.fillStyle = 'rgba(9,12,19,.72)';
  crRoundRect(c, px, py, pw, ph, 10);
  c.strokeStyle = 'rgba(64,224,255,.35)'; c.lineWidth = 1;
  c.stroke();

  // P-numeral: cyan, magenta "/field".
  const pulse = 1 + Math.sin(Math.min(1, CR_POSHUD.flashT / 0.45) * Math.PI) * 0.18;
  const nY = py + 34;
  c.textAlign = 'left';
  c.font = `800 ${Math.round(30 * pulse)}px ui-monospace, monospace`;
  c.shadowColor = '#40e0ff'; c.shadowBlur = 12;
  c.fillStyle = '#40e0ff';
  c.fillText('P' + me.pos, px + 12, nY);
  c.shadowBlur = 0;
  c.font = '700 16px ui-monospace, monospace';
  c.fillStyle = '#ff5ca8';
  c.fillText('/' + CR_RACERS.length, px + 12 + c.measureText('P' + me.pos).width + 26, nY);

  /* BUGFIX 11 · lap line: larger + full-opacity text (was 11px at 75%
   * alpha — illegible at small canvas sizes). */
  c.font = '700 13px ui-monospace, monospace';
  c.fillStyle = 'rgba(240,244,252,.95)';
  const lapTxt = L.finished ? `FINISHED · ${L.totalLaps}/${L.totalLaps}`
                            : `LAP ${L.lap}/${L.totalLaps}`;
  c.fillText(lapTxt, px + 12, py + 52);

  // Gap to racer ahead / behind (progress in gate-units -> metres-ish).
  // With one racer both read '—' honestly (no fabricated opponents).
  const sorted = CR_RACERS;                     // already ranked
  const fmtGap = (gp) => {
    if (gp == null) return '—';
    const m = gp * (CR.SEG_LEN * (g.totalSegs / (g.cps.length)) ) / 100;
    return (m >= 1000 ? (m / 1000).toFixed(1) + 'km' : Math.round(m) + 'm');
  };
  let aheadTxt = '—', behindTxt = '—';
  if (sorted.length > 1) {
    const i = sorted.indexOf(me);
    if (i > 0)        aheadTxt  = '-' + fmtGap(sorted[i - 1].progress - me.progress);
    if (i < sorted.length - 1) behindTxt = '+' + fmtGap(me.progress - sorted[i + 1].progress);
  }
  /* BUGFIX 11 · gap readouts: 12px bold at ~92% opacity (was 11px/85%). */
  c.font = '700 12px ui-monospace, monospace';
  c.fillStyle = 'rgba(124,255,214,.92)';
  c.fillText('AHD ' + aheadTxt, px + 12, py + 67);
  c.fillStyle = 'rgba(255,166,110,.92)';
  c.fillText('BHD ' + behindTxt, px + pw / 2 + 4, py + 67);
  c.restore();

  /* P3.13 · rear warning arrow: when an AI rival is close BEHIND the
   * player on track (same-lap distance, not standings), a subtle amber
   * chevron pulses at the bottom edge under the rival's lane side —
   * enough to notice without stealing attention from the road. */
  {
    const playerSegF = g.position / CR.SEG_LEN;
    let threat = null, threatDz = Infinity;
    for (const r of CR_RACERS) {
      if (r.isPlayer || !r.ai || r.finished) continue;
      let dz = playerSegF - (((r.ai.segF % g.totalSegs) + g.totalSegs) % g.totalSegs);   // >0 = rival behind
      if (dz < -g.totalSegs / 2) dz += g.totalSegs;
      if (dz > g.totalSegs / 2) dz -= g.totalSegs;
      if (dz > 0 && dz < 26 && dz < threatDz) { threat = r; threatDz = dz; }
    }
    if (threat) {
      const strength = 1 - threatDz / 26;                // 0..1 as it closes
      const pulse = g.reducedMotion ? 0.7
                  : 0.55 + 0.45 * Math.sin(performance.now() / 140);
      const ax = W / 2 + (threat.ai.x - g.playerX) * W * 0.30;
      const ay = H - 26;
      c.save();
      c.globalAlpha = (0.30 + 0.45 * strength) * pulse;
      c.strokeStyle = '#ffb056';
      c.lineWidth = 3; c.lineJoin = 'round'; c.lineCap = 'round';
      c.shadowColor = 'rgba(255,176,86,.8)';
      c.shadowBlur = 8;
      c.beginPath();
      c.moveTo(ax - 9, ay - 7);
      c.lineTo(ax, ay + 3);
      c.lineTo(ax + 9, ay - 7);
      c.stroke();
      c.restore();
    }
  }
}

/* TASK 7.2 · compact standings panel: position, name, lap, progress %,
 * finished flag — straight from the live field, ready for AI racers in
 * Task 7.3 (it renders whatever CR_RACERS holds). Collapsed to a slim
 * single-racer strip when only the player is registered. */
function crDrawStandings(c, W, H) {
  const g = CRG, L = CR_LAP;
  if (!L.enabled || !CR_RACERS.length) return;
  const rowH = 16, headH = 15;
  const sw = Math.max(130, Math.min(196, W * 0.25));
  const sh = headH + CR_RACERS.length * rowH + 8;
  /* BUGFIX 11 · standings sit below the position HUD with the same
   * shake-safe 16px left margin (was 12 — clipped at the edge). */
  const sx = 16, sy = 16 + 78 + 8;      // below the position HUD

  c.save();
  c.fillStyle = 'rgba(9,12,19,.66)';
  crRoundRect(c, sx, sy, sw, sh, 8);
  c.strokeStyle = 'rgba(255,92,168,.30)'; c.lineWidth = 1;
  c.stroke();

  /* BUGFIX 11 · standings header: 10px at full opacity (was 9px/90%). */
  c.font = '700 10px ui-monospace, monospace';
  c.fillStyle = 'rgba(178,190,214,.95)';
  c.textAlign = 'left';
  c.fillText('POS NAME           LAP  PROG', sx + 8, sy + 11);

  const totalUnits = L.totalLaps * (g.cps ? g.cps.length : 1);
  for (let i = 0; i < CR_RACERS.length; i++) {
    const r = CR_RACERS[i];
    const y = sy + headH + 5 + i * rowH;
    const isMe = r.isPlayer;
    // Player row: cyan + bright; others magenta-tinted.
    /* BUGFIX 11 · standings rows: 11px at higher opacity (was 10px/80%). */
    c.fillStyle = isMe ? '#5ce8ff' : 'rgba(236,240,248,.92)';
    if (isMe) { c.shadowColor = '#40e0ff'; c.shadowBlur = 6; }
    c.font = isMe ? '700 11px ui-monospace, monospace'
                  : '600 11px ui-monospace, monospace';
    const pos = String(r.pos).padStart(2, ' ');
    const nm = (r.name.length > 12 ? r.name.slice(0, 11) + '…' : r.name)
               .padEnd(13, ' ');
    const lp = (r.finished ? 'F' : String(r.lap)).padStart(3, ' ');
    const pr = (Math.max(0, Math.min(1, r.progress / totalUnits)) * 100)
               .toFixed(0).padStart(3, ' ') + '%';
    c.fillText(`${pos} ${nm}${lp} ${pr}`, sx + 8, y + 9);
    c.shadowBlur = 0;
  }
  c.restore();
}

/** Radial speed lines during high speed / nitro. Length and count both
 *  scale with speed so acceleration is felt, not just read off the HUD. */
function crDrawSpeedLines(c, W, H) {
  const g = CRG;
  const sr = Math.min(1.35, g.speed / g.maxSpeed);
  // Phase 3: deterministic line anchors (hash-seeded) so the field is
  // STABLE frame to frame — the lines grow outward with speed instead of
  // strobing randomly, which reads as true motion blur.
  const n = Math.round((g.nitroActive ? 16 : 8) + sr * 10);
  const cx = W / 2, cy = H * 0.55;
  c.save();
  c.lineWidth = 2;
  for (let i = 0; i < n; i++) {
    const h1 = crHash(i * 7 + 3), h2 = crHash(i * 13 + 11);
    const ang = h1 * Math.PI * 2;
    const r0 = 150 + h2 * 110;
    // Length scales smoothly with actual speed (plus nitro swell).
    const len = (18 + sr * 64) * (0.45 + h1 * 0.75) * (1 + g.nitroVis * 0.5);
    const a = (0.10 + sr * 0.22 + g.nitroVis * 0.20) * (0.5 + h2 * 0.5);
    c.strokeStyle = g.nitroVis > 0.05
      ? `rgba(190,160,255,${a.toFixed(3)})`
      : `rgba(232,236,244,${a.toFixed(3)})`;
    c.beginPath();
    c.moveTo(cx + Math.cos(ang) * r0, cy + Math.sin(ang) * r0);
    c.lineTo(cx + Math.cos(ang) * (r0 + len), cy + Math.sin(ang) * (r0 + len));
    c.stroke();
  }
  c.restore();
}

/** Speed vignette: the screen edges darken and streak as speed rises,
 *  which reads as motion blur without any expensive filter. */
function crDrawSpeedVignette(c, W, H) {
  const g = CRG;
  const sr = Math.min(1, g.speed / g.maxSpeed);
  if (sr < 0.45) return;
  const a = (sr - 0.45) * (0.26 + g.nitroVis * 0.16);
  const vg = c.createRadialGradient(W / 2, H * 0.55, H * 0.28, W / 2, H * 0.55, H * 0.85);
  vg.addColorStop(0, 'rgba(5,7,10,0)');
  vg.addColorStop(1, `rgba(5,7,10,${a.toFixed(3)})`);
  c.fillStyle = vg;
  c.fillRect(0, 0, W, H);
}

/** Phase 3 · Peripheral motion blur (lightweight).
 *  Two soft vertical gradient bands on the left/right screen edges whose
 *  alpha scales with speed — the classic "tunnel vision" blur, done with
 *  two gradients instead of any filter. Costs ~2 fills; skipped at low
 *  speed and under reduced motion. */
function crDrawPeripheralBlur(c, W, H) {
  const g = CRG;
  const sr = Math.min(1.35, g.speed / g.maxSpeed);
  if (sr < 0.55 || g.reducedMotion) return;
  const k = (sr - 0.55) / 0.8;                     // 0..1 across the range
  const a = Math.min(0.34, k * 0.30 + g.nitroVis * 0.10);
  const bw = W * 0.16;
  const lg = c.createLinearGradient(0, 0, bw, 0);
  lg.addColorStop(0, `rgba(12,14,24,${a.toFixed(3)})`);
  lg.addColorStop(1, 'rgba(12,14,24,0)');
  c.fillStyle = lg;
  c.fillRect(0, 0, bw, H);
  const rg = c.createLinearGradient(W, 0, W - bw, 0);
  rg.addColorStop(0, `rgba(12,14,24,${a.toFixed(3)})`);
  rg.addColorStop(1, 'rgba(12,14,24,0)');
  c.fillStyle = rg;
  c.fillRect(W - bw, 0, bw, H);
}

/** Distance fog near the horizon: far segments fade into the sky colour so
 *  the road appears to emerge from the night haze instead of ending hard. */
function crDrawFog(c, v, W) {
  const { p1, p2 } = v;
  // Fog strength grows with distance from the camera.
  /* BUGFIX 8 · fog now starts easing in earlier (0.45 vs 0.62) and with a
   * softer curve, so the far road melts INTO the horizon gradient above
   * it instead of stopping short and leaving a raw tarmac edge. */
  const depth = v.n / CR.DRAW_DIST;
  if (depth < 0.45) return;
  const a = (depth - 0.45) / 0.55;                 // 0..1 in the far half
  /* LIGHTING FIX · fog ceiling from CR_LIGHT (was 0.82): the far third
   * still melts into the night but the distant road GEOMETRY — curves,
   * lane dashes, edge lines — stays legible instead of vanishing. */
  const alpha = Math.min(CR_LIGHT.FOG_MAX, a * a * 0.95);
  if (alpha <= 0.01) return;
  // A violet-blue haze that lands between the sky and the road so distant
  // geometry melts into the night without ever looking like more tarmac.
  c.fillStyle = `rgba(20,17,52,${alpha.toFixed(3)})`;
  // Fog only the tarmac + rumble span of THIS segment — grass and sky keep
  // their own palette, which keeps the horizon readable.
  const r1 = p1.w * 0.16, r2 = p2.w * 0.16;
  crPoly(c,
    p1.x - p1.w - r1, p1.y,  p1.x + p1.w + r1, p1.y,
    p2.x + p2.w + r2, p2.y,  p2.x - p2.w - r2, p2.y);
}

/* ---- 7. HUD ----------------------------------------------------------- */
const CR_HUD_CACHE = {};
function crFmtTime(ms) {
  if (!ms || ms < 0) ms = 0;
  const t = Math.floor(ms / 1000);
  const m = Math.floor(t / 60), s = t % 60;
  return `${m}:${String(s).padStart(2, '0')}`;
}
function crHud() {
  const g = CRG;
  const set = (id, val) => {
    if (CR_HUD_CACHE[id] === val) return;
    CR_HUD_CACHE[id] = val;
    const e = $(id); if (e) e.textContent = val;
  };
  set('#crScore', String(Math.round(g.score)));
  set('#crSpeed', String(Math.round(g.speed / 100)));       // "km/h" feel
  set('#crTime', crFmtTime(g.raceTimeMs));                  // PHASE 4 timer
  set('#crDist', (g.distance / 100000).toFixed(2) + ' km'); // PHASE 4 odometer
  /* TASK 7.1 · lap indicator: "2/3" for lap races, plain circuit count
   * otherwise (endless/timeattack keep their odometer semantics). */
  set('#crLap', CR_LAP.enabled
      ? (CR_LAP.finished ? `${CR_LAP.totalLaps}/${CR_LAP.totalLaps}`
                         : `${CR_LAP.lap}/${CR_LAP.totalLaps}`)
      : String(g.laps + 1));
  set('#crPassed', String(g.passed));
  set('#crNear', String(g.nearMisses));                     // PHASE 4
  set('#crBest', String(g.highScore));

  /* PHASE 4 · combo chain read-out: visible only while a chain is live,
   * with the remaining window shown as an opacity fade on the element. */
  const cbEl = $('#crCombo');
  if (cbEl) {
    const live = g.combo > 1 && g.comboT > 0;
    const cv = live ? `×${g.combo} COMBO` : '';
    if (CR_HUD_CACHE.combo !== cv) {
      CR_HUD_CACHE.combo = cv;
      cbEl.textContent = cv;
      cbEl.classList.toggle('hidden', !live);
    }
    // Window fade (cheap: style write only when meaningfully changed).
    if (live) {
      const f = Math.min(1, g.comboT / 1.2).toFixed(2);
      if (CR_HUD_CACHE.comboF !== f) { CR_HUD_CACHE.comboF = f; cbEl.style.opacity = f; }
    } else if (CR_HUD_CACHE.comboF !== '') {
      CR_HUD_CACHE.comboF = ''; cbEl.style.opacity = '';
    }
  }

  // Live drift read-out: shows accumulating drift points while sliding, so
  // the player sees the payout building in real time (not just at the end).
  const drEl = $('#crDrift');
  if (drEl) {
    const active = g.drifting && g.driftScore >= 10;
    const val = active ? `DRIFT ${Math.round(g.driftScore)}` : '';
    if (CR_HUD_CACHE.drift !== val) {
      CR_HUD_CACHE.drift = val;
      drEl.textContent = val;
      drEl.classList.toggle('on', active);
    }
  }

  // Mode-specific stat.
  if (g.mode === 'timeattack') {
    const left = Math.max(0, g.timeLeftMs);
    set('#crModeK', 'time');
    set('#crModeV', crFmtTime(left));
    const el = $('#crModeStat');
    if (el) el.classList.toggle('warn', left < 10000);
  } else if (CR_LAP.enabled) {
    /* TASK 7.1 · live lap timing: current lap elapsed, with the best lap
     * of this race alongside once one exists. */
    set('#crModeK', 'lap time');
    const cur = CR_LAP.finished ? CR_LAP.prevLapMs
                                : Math.max(0, g.raceTimeMs - g.lapStart);
    set('#crModeV', CR_LAP.bestLapMs
      ? `${crFmtTime(cur)} · best ${crFmtTime(CR_LAP.bestLapMs)}`
      : crFmtTime(cur));
  } else {
    set('#crModeK', 'goal');
    set('#crModeV', `${Math.round(g.score)}/${g.targetScore}`);
  }

  // Nitro meter width.
  const fill = $('#crMeterFill');
  if (fill) {
    const pct = Math.round(g.nitro * 100);
    if (CR_HUD_CACHE.nitro !== pct) {
      CR_HUD_CACHE.nitro = pct;
      fill.style.width = pct + '%';
      const row = $('#crMeterRow');
      if (row) row.classList.toggle('ready', g.nitro >= 0.999);
    }
  }
}

/* =====================================================================
   PHASE 6 · MENU / GARAGE / RECORDS / CHAMPIONSHIP screens.
   One router: crScreen(name) shows exactly one overlay child; the classic
   ready/paused/over flow keeps its own ids so Phase 4 logic is untouched.
   ===================================================================== */
const CR_SCREENS = ['crMenuScreen','crDiffScreen','crGarageScreen',
                    'crRecordsScreen','crChampScreen','crPlayScreen'];
/* FULLSCREEN FIX · the menu screens are the five attract-flow children
 * (everything except crPlayScreen). While one of them is visible the
 * game is in "menu mode": the DOM racing HUD + nitro meter are hidden
 * and the canvas draws ONLY the road, skyline and scenery — no player
 * car, no traffic, no AI racers, no race instruments. The classic
 * ready/pause/over flow (crPlayScreen) is NOT menu mode: the full race
 * scene and HUD render there. */
const CR_MENU_SCREENS = new Set(['crMenuScreen','crDiffScreen',
                                 'crGarageScreen','crRecordsScreen',
                                 'crChampScreen']);
/** True while a menu/attract screen is the active overlay.
 *  Fast path: CRG.overlay is kept in sync by crScreen()/crOverlay()
 *  ('menu' for any attract screen, classic kinds otherwise). The DOM
 *  walk is only a safety net for untracked states. */
function crMenuMode() {
  const o = CRG.overlay;
  if (o === 'menu') return true;
  if (o === 'ready' || o === 'paused' || o === 'over' || o === null) return false;
  const ov = document.getElementById('crOverlay');
  if (!ov || ov.classList.contains('hidden')) return false;
  for (const id of CR_MENU_SCREENS) {
    const el = document.getElementById(id);
    if (el && !el.classList.contains('hidden')) return true;
  }
  return false;
}
function crScreen(name) {
  const ov = $('#crOverlay'); if (!ov) return;
  for (const id of CR_SCREENS) {
    const el = document.getElementById(id);
    if (el) el.classList.toggle('hidden', id !== name);
  }
  ov.classList.remove('hidden');
  ov.setAttribute('aria-hidden', 'false');
  /* FULLSCREEN FIX · sync panel-wide menu state + scrim weight:
   * menu screens get the light scrim, the play flow the heavy one. */
  const menu = CR_MENU_SCREENS.has(name);
  ov.classList.toggle('cr-scrim-heavy', !menu);
  const panel = $('#crPanel');
  if (panel) panel.classList.toggle('cr-menu-on', menu);
  if (menu) CRG.overlay = 'menu';
}
function crScreenHide() {
  const ov = $('#crOverlay'); if (!ov) return;
  ov.classList.add('hidden');
  ov.setAttribute('aria-hidden', 'true');
  /* Leaving the overlay always means leaving menu mode: the race HUD,
   * nitro meter and every race sprite come back immediately. */
  ov.classList.remove('cr-scrim-heavy');
  const panel = $('#crPanel');
  if (panel) panel.classList.remove('cr-menu-on');
}

/** Small canvas preview of a car body (used in garage cards). Draws the
 *  same rear-view silhouette family as gameplay, scaled to the card. */
function crCarThumb(canvas, carId, cust) {
  const c = canvas.getContext('2d');
  const W = canvas.width, H = canvas.height;
  c.clearRect(0, 0, W, H);
  const def = CR_CARS.find(x => x.id === carId) || CR_CARS[0];
  const BODY = {
    gt:      { w: .78, h: .50, roof: 1.00 },
    wedge:   { w: .74, h: .44, roof: 0.90 },
    drift:   { w: .80, h: .48, roof: 0.97 },
    muscle:  { w: .86, h: .53, roof: 1.05 },
    phantom: { w: .82, h: .47, roof: 0.93 },
  }[def.body] || { w: .78, h: .50, roof: 1 };
  const cw = W * BODY.w, ch = H * BODY.h;
  const cx = W / 2, cy = H * 0.72;
  const hw = cw / 2;
  // shadow
  c.fillStyle = 'rgba(0,0,0,.5)';
  c.beginPath(); c.ellipse(cx, cy + 3, cw * .55, 8, 0, 0, Math.PI * 2); c.fill();
  // wheels
  c.fillStyle = '#07090e';
  for (const ox of [-hw + cw*.14, hw - cw*.14]) {
    c.beginPath(); c.arc(cx + ox, cy - ch*.12, ch*.16, 0, Math.PI*2); c.fill();
  }
  // body
  const g = c.createLinearGradient(0, cy - ch, 0, cy);
  g.addColorStop(0, crShade(cust.body, .5));
  g.addColorStop(.6, cust.body);
  g.addColorStop(1, crShade(cust.body, -.5));
  c.fillStyle = g;
  c.beginPath();
  c.moveTo(cx - hw, cy - ch*.10);
  c.lineTo(cx - hw + 4, cy - ch*.60);
  c.quadraticCurveTo(cx, cy - ch*.68, cx + hw - 4, cy - ch*.60);
  c.lineTo(cx + hw, cy - ch*.10);
  c.quadraticCurveTo(cx + hw*.9, cy + 2, cx + hw*.7, cy + 3);
  c.lineTo(cx - hw*.7, cy + 3);
  c.quadraticCurveTo(cx - hw*.9, cy + 2, cx - hw, cy - ch*.10);
  c.closePath(); c.fill();
  // cabin
  c.fillStyle = 'rgba(8,11,19,.92)';
  c.beginPath();
  c.moveTo(cx - hw*.58, cy - ch*.58);
  c.quadraticCurveTo(cx, cy - ch*(.95*BODY.roof+.05), cx + hw*.58, cy - ch*.58);
  c.closePath(); c.fill();
  // spoiler hint
  const sp = CR_SPOILERS.find(s => s.id === cust.spoiler);
  if (sp && sp.h > 0) {
    c.fillStyle = '#10161f';
    c.fillRect(cx - hw*.85, cy - ch*(0.62 + sp.h*3), cw*.85*2*0+ cw*.7, 4);
  }
  // neon underglow
  const gg = c.createRadialGradient(cx, cy, 4, cx, cy, cw*.5);
  gg.addColorStop(0, cust.neon); gg.addColorStop(1, 'rgba(0,0,0,0)');
  c.save(); c.globalCompositeOperation='lighter'; c.globalAlpha=.25;
  c.fillStyle = gg;
  c.beginPath(); c.ellipse(cx, cy, cw*.55, 10, 0, 0, Math.PI*2); c.fill();
  c.restore();
}

/** Render the garage: car grid + customization panel. */
function crGarageRender(sel) {
  const grid = $('#crCarGrid'); if (!grid) return;
  const selId = sel || CRG.carId || CR_SAVE.car;
  grid.innerHTML = '';
  for (const car of CR_CARS) {
    const unlocked = crUnlocked(car);
    const card = document.createElement('button');
    card.type = 'button';
    card.className = 'cr-car-card' + (car.id === selId ? ' sel' : '')
                   + (unlocked ? '' : ' locked');
    card.dataset.car = car.id;
    const stats = [
      ['accel', car.accel], ['top', car.top],
      ['steer', car.steer], ['grip', car.grip], ['drift', car.drift],
    ];
    card.innerHTML = `
      <span class="cr-car-name">${esc(car.name)}</span>
      <canvas width="120" height="64" aria-hidden="true"></canvas>
      <span class="cr-car-desc">${esc(car.desc)}</span>
      <span class="cr-car-stats">${stats.map(([k, v]) =>
        `<i title="${k}"><b>${k}</b><span style="width:${Math.round(Math.min(1.3, v)/1.3*100)}%"></span></i>`
      ).join('')}</span>
      ${unlocked ? '' : `<span class="cr-car-lock">🔒 ${esc(car.unlock.text)}</span>`}`;
    card.addEventListener('click', () => {
      if (!unlocked) { crSfx('ui'); return; }
      CRG.carId = car.id; CR_SAVE.car = car.id; crSaveWrite();
      crSfx('ui'); crGarageRender(car.id); crGarageCustomRender();
    });
    grid.appendChild(card);
    crCarThumb(card.querySelector('canvas'), car.id,
               unlocked ? CR_SAVE.custom : { ...CR_SAVE.custom, body:'#39404d', neon:'#39404d' });
  }
  crGarageCustomRender();
}

/** Customization panel: body colour, neon, wheels, spoiler, plate. */
function crGarageCustomRender() {
  const box = $('#crCustomBox'); if (!box) return;
  const cu = CR_SAVE.custom;
  const wheelOpts = CR_WHEELS.map(w => {
    const ok = crUnlocked(w);
    return `<button type="button" data-w="${w.id}" class="${cu.wheel===w.id?'sel':''}${ok?'':' locked'}"
      title="${esc(w.name)}${ok?'':' — '+esc(w.unlock.text)}">${esc(w.name)}${ok?'':' 🔒'}</button>`;
  }).join('');
  const spOpts = CR_SPOILERS.map(s => {
    const ok = crUnlocked(s);
    return `<button type="button" data-s="${s.id}" class="${cu.spoiler===s.id?'sel':''}${ok?'':' locked'}"
      title="${esc(s.name)}${ok?'':' — '+esc(s.unlock.text)}">${esc(s.name)}${ok?'':' 🔒'}</button>`;
  }).join('');
  const neonOpts = CR_NEONS.map(n =>
    `<button type="button" data-n="${n}" class="cr-neon-sw ${cu.neon===n?'sel':''}"
      style="background:${n}" aria-label="neon ${n}"></button>`).join('');
  const bodyOpts = ['#49e0b0','#ff5ca8','#6fb7ff','#ffc857','#b48cff','#ff6b5e','#e8ecf4','#2ee6a8']
    .map(n => `<button type="button" data-b="${n}" class="cr-neon-sw ${cu.body===n?'sel':''}"
      style="background:${n}" aria-label="paint ${n}"></button>`).join('');
  box.innerHTML = `
    <div class="cr-cust-row"><label>paint</label><span class="cr-sw-row">${bodyOpts}</span></div>
    <div class="cr-cust-row"><label>neon</label><span class="cr-sw-row">${neonOpts}</span></div>
    <div class="cr-cust-row"><label>wheels</label><span class="cr-opt-row">${wheelOpts}</span></div>
    <div class="cr-cust-row"><label>spoiler</label><span class="cr-opt-row">${spOpts}</span></div>
    <div class="cr-cust-row"><label>plate</label>
      <input id="crPlateIn" maxlength="8" value="${esc(cu.plate)}" spellcheck="false"></div>`;
  // wire events
  box.querySelectorAll('[data-b]').forEach(b => b.addEventListener('click', () => {
    CR_SAVE.custom.body = b.dataset.b; crSaveWrite(); crSfx('ui');
    crGarageCustomRender(); crGarageRender();
  }));
  box.querySelectorAll('[data-n]').forEach(b => b.addEventListener('click', () => {
    CR_SAVE.custom.neon = b.dataset.n; crSaveWrite(); crSfx('ui');
    crGarageCustomRender(); crGarageRender();
  }));
  box.querySelectorAll('[data-w]').forEach(b => b.addEventListener('click', () => {
    const w = CR_WHEELS.find(x => x.id === b.dataset.w);
    if (!crUnlocked(w)) { crSfx('ui'); return; }
    CR_SAVE.custom.wheel = w.id; crSaveWrite(); crSfx('ui'); crGarageCustomRender();
  }));
  box.querySelectorAll('[data-s]').forEach(b => b.addEventListener('click', () => {
    const s = CR_SPOILERS.find(x => x.id === b.dataset.s);
    if (!crUnlocked(s)) { crSfx('ui'); return; }
    CR_SAVE.custom.spoiler = s.id; crSaveWrite(); crSfx('ui'); crGarageCustomRender();
  }));
  const pin = $('#crPlateIn');
  if (pin) pin.addEventListener('change', () => {
    CR_SAVE.custom.plate = (pin.value || 'SYN-01').toUpperCase().replace(/[^A-Z0-9 -]/g,'').slice(0,8) || 'SYN-01';
    crSaveWrite(); pin.value = CR_SAVE.custom.plate;
  });
}

/** Refresh lock badges after a race (unlocks may have changed). */
function crGarageRefreshLocks() {
  if ($('#crGarageScreen') && !$('#crGarageScreen').classList.contains('hidden')) {
    crGarageRender();
  }
}

/** Records + achievements screen. */
function crRecordsRender() {
  const grid = $('#crRecGrid'); if (!grid) return;
  const r = CR_SAVE.records;
  const rows = [
    ['highest score', r.highScore.toLocaleString()],
    ['longest distance', r.longestKm.toFixed(2) + ' km'],
    ['highest speed', r.topKmh + ' km/h'],
    ['best drift', r.bestDrift.toLocaleString()],
    ['most near misses', r.mostNear],
    ['cleanest race', r.cleanestKm.toFixed(2) + ' km'],
    ['best championship', r.bestChampPts + ' pts'],
    ['races finished', CR_SAVE.totals.races],
  ];
  grid.innerHTML = rows.map(([k, v]) =>
    `<div class="cr-rec"><span class="k">${k}</span><span class="v">${v}</span></div>`).join('');
  const list = $('#crAchList'); if (!list) return;
  list.innerHTML = CR_ACHIEVEMENTS.map(a => {
    const got = CR_SAVE.achievements.includes(a.id);
    return `<div class="cr-ach ${got ? 'got' : ''}">
      <span class="ico">${got ? a.icon : '🔒'}</span>
      <span class="tx"><b>${esc(a.name)}</b><i>${esc(a.text)}</i></span>
    </div>`;
  }).join('');
}

/** Championship screen: progress, standings, next event, restart. */
function crChampRender() {
  const body = $('#crChampBody'); if (!body) return;
  const acts = $('#crChampActions');
  const ch = CR_SAVE.champ;
  const hist = CR_SAVE.champHistory;
  let html = '';
  if (!ch) {
    html += `<div class="cr-champ-new">
      <p>Four events. Points from every finish. Best total takes the crown.</p>
      <ol class="cr-champ-evs">${CR_CHAMP_EVENTS.map((e, i) =>
        `<li><b>${i+1}. ${esc(e.name)}</b> — ${esc(CR_MODES[e.mode].label)} · ${esc(CR_DIFFS[e.diff].label)}</li>`).join('')}
      </ol></div>`;
    if (hist.length) {
      html += `<h4>completed championships</h4><table class="cr-champ-table">
        <thead><tr><th>date</th><th>result</th><th>points</th></tr></thead><tbody>
        ${hist.slice(0, 5).map(h => `<tr><td>${esc(h.name)}</td>
          <td>${h.wonByPlayer ? '🏆 won' : '— ' + (h.standings.findIndex(s=>s.you)+1) + ordinal(h.standings.findIndex(s=>s.you)+1)}</td>
          <td>${h.points}</td></tr>`).join('')}</tbody></table>`;
    }
    if (acts) acts.innerHTML =
      `<button type="button" class="ft-btn primary" id="crChampStart">Start Championship</button>
       <button type="button" class="ft-btn cr-back" id="crChampBack">← Back</button>`;
  } else {
    html += `<div class="cr-champ-live">
      <div class="cr-champ-pts">your points: <b>${ch.points}</b> · event ${Math.min(ch.eventIdx+1, ch.events.length)}/${ch.events.length}</div>
      <ol class="cr-champ-evs">${ch.events.map((e, i) => {
        const done = e.done;
        const res = e.result;
        return `<li class="${done ? 'done' : i === ch.eventIdx ? 'next' : ''}">
          <b>${i+1}. ${esc(e.name)}</b> — ${esc(CR_MODES[e.mode].label)} · ${esc(CR_DIFFS[e.diff].label)}
          ${done && res ? `<span class="res">${res.score} pts · ${res.rating}${res.won ? ' · won' : ''}</span>` : ''}
        </li>`;
      }).join('')}</ol>
      <p class="cr-champ-note">Progress is saved automatically — leave and come back any time.</p>
    </div>`;
    if (acts) acts.innerHTML =
      `<button type="button" class="ft-btn primary" id="crChampNext">${ch.eventIdx >= ch.events.length ? 'See Final Results' : 'Race Next Event'}</button>
       <button type="button" class="ft-btn" id="crChampAbandon">Restart Championship</button>
       <button type="button" class="ft-btn cr-back" id="crChampBack">← Back</button>`;
  }
  body.innerHTML = html;
  // actions
  const start = $('#crChampStart');
  if (start) start.addEventListener('click', () => {
    CR_SAVE.champ = crChampNew(); crSaveWrite(); crSfx('ui'); crChampRender();
  });
  const next = $('#crChampNext');
  if (next) next.addEventListener('click', () => {
    if (!CR_SAVE.champ) return;
    if (CR_SAVE.champ.eventIdx >= CR_SAVE.champ.events.length) { crChampRender(); return; }
    const ev = CR_SAVE.champ.events[CR_SAVE.champ.eventIdx];
    crLaunchRace(ev.mode, ev.diff, true);
  });
  const ab = $('#crChampAbandon');
  if (ab) ab.addEventListener('click', () => {
    CR_SAVE.champ = crChampNew(); crSaveWrite(); crSfx('ui'); crChampRender();
  });
  const back = $('#crChampBack');
  if (back) back.addEventListener('click', () => { crSfx('ui'); crShowMenu(); });
}
function ordinal(n) { const s = ['th','st','nd','rd'], v = n % 100; return s[(v-20)%10] || s[v] || s[0]; }

/** Menu footer: totals + best score. */
function crMenuFootRender() {
  const el = $('#crMenuFoot'); if (!el) return;
  el.innerHTML = `best ${CR_SAVE.records.highScore.toLocaleString()} · `
    + `${CR_SAVE.totals.races} races · ${CR_SAVE.totals.distanceKm.toFixed(1)} km driven`;
}

/** Show the main menu (also resets any live race state). */
function crShowMenu() {
  crScreen('crMenuScreen');
  crMenuFootRender();
  CRG.overlay = 'menu';
}

/** Launch a race with mode+difficulty (from menu or championship). */
function crLaunchRace(mode, diff, champRun) {
  // Snapshot unlocks so post-race diffing can detect NEW unlocks.
  CRG._unlockedBefore = [
    ...CR_CARS.filter(c=>crUnlocked(c)).map(c=>c.id),
    ...CR_WHEELS.filter(w=>crUnlocked(w)).map(w=>'w:'+w.id),
    ...CR_SPOILERS.filter(s=>crUnlocked(s)).map(s=>'s:'+s.id),
  ];
  const fresh = crNewGame(mode, diff);
  for (const k of Object.keys(fresh)) CRG[k] = fresh[k];
  CRG.carId = CR_SAVE.car;
  CRG.champRun = !!champRun;
  CRG.keys = Object.create(null);
  crTouchReset();
  const D = CR_DIFFS[diff] || CR_DIFFS.medium;
  CRG.traffic = crSpawnTraffic(D.traffic);
  CR_STREAKS.length = 0; CR_SPARKS.length = 0; crFxClear();
  if (CRG.smoke) CRG.smoke.length = 0;
  if (CRG.skids) CRG.skids.length = 0;
  for (const k of Object.keys(CR_HUD_CACHE)) delete CR_HUD_CACHE[k];
  crScreenHide();
  CRG.overlay = null;
  crHud();
  // straight into the countdown
  CRG.state = 'countdown';
  CRG.countdown = 3.99;
  crSfx('count');
}

/* ---- 8. Overlay ------------------------------------------------------- */
function crOverlay(kind) {
  const ov = $('#crOverlay'); if (!ov) return;
  CRG.overlay = kind;
  /* FULLSCREEN/VIEWPORT FIX · dismissing the overlay always leaves menu
   * mode: if a menu screen was up when the race started (ready-overlay
   * button or the CRD.start() hook), the panel kept `cr-menu-on` and the
   * DOM HUD strip + nitro meter stayed display:none for the whole race.
   * Clearing it here keeps every launch path consistent with
   * crLaunchRace()'s crScreenHide(). */
  if (!kind) {
    ov.classList.add('hidden'); ov.setAttribute('aria-hidden', 'true');
    const panel = $('#crPanel');
    if (panel) panel.classList.remove('cr-menu-on');
    return;
  }
  ov.classList.remove('hidden'); ov.setAttribute('aria-hidden', 'false');
  // Phase 6: classic overlays live on the play screen.
  crScreen('crPlayScreen');

  const r = CRG.result;
  const M = {
    ready: ['Cyber Rush',
            'Steer with ←/→ or A/D · ↑/W to accelerate · ↓/S to brake · Space/N for nitro · hold Shift while steering to drift (Shift drift)',
            'Start Engine'],
    paused: ['Paused', 'Press P or Resume to continue', 'Resume'],
    over: [r && r.won ? 'Race Complete' : 'Game Over', null, 'Play Again'],
  };
  const [title, sub, btn] = M[kind] || M.ready;
  $('#crOvTitle').textContent = title;
  $('#crOvSub').textContent =
    kind === 'over' && r
      ? (r.won
          ? (r.raceWonByLaps
              ? `All ${r.totalLaps} laps complete in ${crFmtTime(r.time)} · best lap ${crFmtTime(r.bestLapMs)}`
              : `Target reached in ${crFmtTime(r.time)}${r.bestLapMs ? ` · best lap ${crFmtTime(r.bestLapMs)}` : ''}`)
          : (CRG.mode === 'timeattack' ? 'Time expired.' : 'Keep pushing — try again.'))
      : (sub || '');
  $('#crOvBtn').textContent = btn;
  $('#crOvBtn').dataset.kind = kind;

  // Result stats grid only on game over.
  const stats = $('#crOvStats');
  if (stats) {
    if (kind === 'over' && r) {
      stats.classList.remove('hidden');
      const put = (id, v) => { const e = $(id); if (e) e.textContent = v; };
      put('#crRsScore', String(r.score));
      put('#crRsDist', `${r.distanceKm.toFixed(2)} km`);
      put('#crRsTime', crFmtTime(r.time));
      put('#crRsMax', `${r.maxKmh} km/h`);
      put('#crRsDrift', String(r.bestDrift));
      put('#crRsNear', String(r.nearMisses));
      put('#crRsNitro', `${r.nitroUsedS.toFixed(1)}s`);
      put('#crRsPassed', String(r.passed));
      put('#crRsCrashes', String(r.crashes));
      const posEl = $('#crRsPos');
      if (posEl) {
        const hasField = r.fieldSize > 0;
        posEl.parentElement.classList.toggle('hidden', !hasField);
        put('#crRsPos', hasField ? `P${r.finishPos} of ${r.fieldSize}` : '—');
      }
      put('#crRsRating', r.rating);
      const bestEl = $('#crRsBestTag');
      if (bestEl) bestEl.classList.toggle('hidden', !r.best);
      /* PHASE 6 · progression summary under the stats: new achievements,
       * record improvements, championship standing. */
      const prog = $('#crOvProgress');
      if (prog) {
        const bits = [];
        if (r.improved && r.improved.length)
          bits.push(`<span class="cr-prog rec">★ new ${r.improved.join(' & ')} record</span>`);
        for (const a of (r.newAchievements || []))
          bits.push(`<span class="cr-prog ach">${a.icon} achievement: ${esc(a.name)}</span>`);
        for (const u of (r.unlocked || []))
          bits.push(`<span class="cr-prog unlk">🔓 unlocked: ${esc(u.name)}</span>`);
        if (r.champ) {
          if (r.champ.finished && r.champ.standings) {
            const pos = r.champ.standings.findIndex(s => s.you) + 1;
            bits.push(`<span class="cr-prog champ">🏆 championship ${ordinal(pos)} — ${pos === 1 ? 'you win!' : 'better luck next time'}</span>`);
          } else {
            bits.push('<span class="cr-prog champ">🏁 championship event complete — points banked</span>');
          }
        }
        prog.innerHTML = bits.join('');
        prog.classList.toggle('hidden', bits.length === 0);
      }
    } else {
      stats.classList.add('hidden');
      const prog = $('#crOvProgress');
      if (prog) { prog.classList.add('hidden'); prog.innerHTML = ''; }
    }
  }
}

/* ---- 9. Actions ------------------------------------------------------- */
function crStart() {
  const g = CRG;
  if (g.state === 'ready' || g.state === 'paused') {
    if (g.state === 'ready') {
      g.state = 'countdown';
      g.countdown = 3.99;
      crOverlay(null);
      crSfx('count');
    } else {
      g.state = 'playing';
      crOverlay(null);
      if (CRA.on) crEngineStart();
    }
  }
}
function crPause() {
  const g = CRG;
  if (g.state === 'playing' || g.state === 'countdown') {
    g.state = 'paused';
    crEngineStop();
    crOverlay('paused');
  }
}
function crTogglePause() { CRG.state === 'paused' ? crStart() : crPause(); }
function crRestart() {
  const keep = CRG.highScore;
  const keepMode = CRG.mode;
  const keepDiff = CRG.diff || 'medium';
  const keepChamp = CRG.champRun;
  // Preserve the canvas binding: crNewGame returns a template, so copy
  // field-by-field rather than Object.assign over live refs.
  const fresh = crNewGame(keepMode, keepDiff);
  for (const k of Object.keys(fresh)) CRG[k] = fresh[k];
  // A fresh race must never inherit held input or stale cooldowns: release
  // the keyboard/touch state so the countdown starts from a clean slate.
  CRG.keys = Object.create(null);
  CRG.touch = { left:false, right:false, gas:false, brake:false, nitro:false, drift:false };
  CRG.champRun = keepChamp;
  CRG.traffic = crSpawnTraffic((CR_DIFFS[keepDiff] || CR_DIFFS.medium).traffic);
  // Screen-space AND world-anchored effects must not survive a restart.
  CR_STREAKS.length = 0;
  CR_SPARKS.length = 0;
  CRG.smoke.length = 0;                            // tyre-smoke puffs
  CRG.skids.length = 0;                            // skid marks
  crFxClear();                                     // Phase 3 pools
  for (const k of Object.keys(CR_HUD_CACHE)) delete CR_HUD_CACHE[k];
  crOverlay('ready');
  crHud();
  crDraw();
}
/** PHASE 6 · canonical touch-state reset for new lifecycle sites
 *  (race launch, menu return). The original three reset points (restart,
 *  blur, teardown) keep their inline literals. */
function crTouchReset() {
  const T = CRG.touch;
  T.left = false; T.right = false; T.gas = false;
  T.brake = false; T.nitro = false; T.drift = false;
}
/** PHASE 6 · back to menu: reset run state, show the menu screen. */
function crRestartToMenu() {
  const fresh = crNewGame('quick', 'medium');
  for (const k of Object.keys(fresh)) CRG[k] = fresh[k];
  CRG.keys = Object.create(null);
  crTouchReset();
  CRG.traffic = crSpawnTraffic(CR_DIFFS.medium.traffic);
  CR_STREAKS.length = 0; CR_SPARKS.length = 0; crFxClear();
  if (CRG.smoke) CRG.smoke.length = 0;
  if (CRG.skids) CRG.skids.length = 0;
  for (const k of Object.keys(CR_HUD_CACHE)) delete CR_HUD_CACHE[k];
  crEngineStop();
  crShowMenu();
  crHud();
  crDraw();
}

/** Legacy helper kept for compatibility (R key / Restart button): replays
 *  the current mode+difficulty immediately. */
function crSetMode(mode) {
  CRG.mode = mode;
  crRestart();
}

/* ---- 10. Frame loop --------------------------------------------------- */
function crFrame(t) {
  if (CRG.epoch !== CRG.myEpoch) return;       // stale loop: stop silently
  const g = CRG;
  requestAnimationFrame(crFrame);
  window.CRD && window.CRD.tick(t);
  const dtRaw = (t - g.lastT) / 1000;
  const dt = Math.min(Math.max(dtRaw, 0), 0.05);   // clamp after tab suspension
  g.lastT = t;
  // Phase 3: screen-space FX freeze with the game — a paused frame must be
  // a still frame, so particles only advance while actually playing.
  g._dtDraw = g.state === 'playing' ? dt : 0;

  if (g.state === 'countdown') {
    g.countdown -= dt;
    const prev = Math.ceil(g.countdown + dt);
    const now = Math.ceil(g.countdown);
    if (now !== prev && now >= 1 && now <= 3) crSfx('count');
    if (g.countdown <= 1 && prev > 1) crSfx('go');
    /* PHASE 4 · cinematic launch: the FOV is drawn slightly tight during
     * the countdown and eases out to the racing value right as GO fires,
     * which reads as the camera "settling onto" the car. Purely visual —
     * crStep owns the real FOV once playing begins. */
    const launchPull = Math.min(1, g.countdown / 3.99) * 6;
    g.fov = CR.FOV - launchPull;
    if (g.countdown <= 0) {
      g.state = 'playing';
      g.raceTimeMs = 0;                    // timer starts NOW, not earlier
      g.lapStart = 0;
      if (CRA.on) crEngineStart();
    }
  } else if (g.state === 'playing' && CR_LAP.enabled && CR_LAP.finished) {
    /* TASK 7.1 · race finished: freeze progression immediately. The
     * finish banner still animates (render-side timer), the car coasts
     * visually via a short decel, but no further scoring/laps/traffic
     * logic runs. crFinish fires on the NEXT tick from crStep's return —
     * this guard covers the gap frames while it transitions. */
    g.speed = Math.max(0, g.speed - dt * 6000);
    g.position += g.speed * dt;
    /* BUGFIX 5 · keep the transmission honest while coasting to a stop:
     * the gear/RPM model must track the decaying speed or the cluster
     * shows gear 6 on a stationary car. */
    crGearModel(g, dt, false);
    crEffectsStep(dt);
    if (CR_LAP.cpFlash > 0) CR_LAP.cpFlash = Math.max(0, CR_LAP.cpFlash - dt);
    if (CR_LAP.lapFlash > 0) CR_LAP.lapFlash = Math.max(0, CR_LAP.lapFlash - dt);
  } else if (g.state === 'playing') {
    // Phase 3: impact slow-motion scales the SIMULATION clock only — the
    // render loop, HUD and audio all keep running at full rate.
    let simDt = dt;
    if (g.slowmo > 0) {
      g.slowmo = Math.max(0, g.slowmo - dt);
      // Ease back up to full speed over the window (no hard snap back).
      const rec = g.slowmo / CRP.COLLISION_SLOWMO_T;
      simDt = dt * (CRP.COLLISION_SLOWMO_K + (1 - CRP.COLLISION_SLOWMO_K) * (1 - rec));
    }
    crStep(simDt);
  }

  crDraw();
  crHud();
}

/* ---- 11. Canvas init + resize ---------------------------------------- */
function crInit(canvas) {
  CRG.ctx = canvas.getContext('2d');
  CRG.canvas = canvas;
  crResize();
}
/* FULLSCREEN/VIEWPORT FIX · one place decides the canvas box.
 *
 * Fullscreen (native Fullscreen API on #crPanel, or the ft-immersive CSS
 * fallback): the panel IS the screen — no margins, borders or padding
 * (see the body.cr-fs rules in console.html) — so the stage is the flex:1
 * row holding every pixel not used by the compact HUD strip + nitro
 * meter. The canvas is sized to that exact box.
 *
 * Aspect handling (the important part): the design HEIGHT stays fixed at
 * CR.H logical units and the live logical WIDTH (CR.vW) is derived from
 * the real stage aspect, clamped to [VW_MIN, VW_MAX]. Rendering uses a
 * UNIFORM scale on those units, so:
 *   • wider screens see more city/verge at identical perspective,
 *   • narrower screens still show the full road,
 *   • nothing is stretched, cropped or letterboxed,
 *   • physics/collision (world units) are completely unaffected.
 *
 * Windowed: the stage keeps its height-aware cap; the canvas matches it
 * and vW settles near the 960 design width. */
function crResize() {
  const canvas = CRG.canvas; if (!canvas) return;
  const wrap = canvas.parentElement;
  let cssW, cssH;
  if (crIsFullscreen()) {
    /* The stage is the flex:1 row of the fullscreen panel, so its client
     * box is exactly the space left for the game. Measure it directly —
     * never innerWidth/innerHeight, which can lag a transition — and fall
     * back to the visual viewport only if layout is not ready. */
    const sw = wrap.clientWidth, sh = wrap.clientHeight;
    if (sw > 40 && sh > 40) {
      cssW = sw; cssH = sh;
    } else {
      cssW = Math.max(120, window.innerWidth);
      cssH = Math.max(120, window.innerHeight);
    }
  } else {
    cssW = wrap.clientWidth;
    cssH = Math.round(cssW * (CR.H / CR.W));
  }
  cssW = Math.max(120, Math.floor(cssW));
  cssH = Math.max(80, Math.floor(cssH));
  const dpr = window.devicePixelRatio || 1;
  /* FULLSCREEN/VIEWPORT FIX · live logical width: keep the design HEIGHT
   * (CR.H) and derive the width from the real stage aspect, clamped to
   * [VW_MIN, VW_MAX]. Rendering uses a UNIFORM scale on these units, so:
   *   • wider screens see more city/verge at identical perspective,
   *   • narrower screens still show the full road,
   *   • nothing is stretched, cropped or letterboxed,
   *   • physics/collision (world units) are completely unaffected.
   * Windowed, the stage keeps the 16:10 design aspect, so vW settles back
   * near 960 and the classic look is preserved. */
  const vW = Math.max(CR.VW_MIN,
                Math.min(CR.VW_MAX, Math.round((cssW / cssH) * CR.H)));
  CR.vW = vW; CR.vH = CR.H;
  /* Allocation guard: skip the buffer reallocation when the CSS size and
   * dpr are unchanged (resize events often fire with identical geometry —
   * reallocating would churn memory for nothing). */
  if (cssW !== CRG.cssW || cssH !== CRG.cssH || dpr !== CRG.dpr
      || canvas.width !== Math.round(cssW * dpr)) {
    canvas.style.width = cssW + 'px';
    canvas.style.height = cssH + 'px';
    canvas.width = Math.round(cssW * dpr);
    canvas.height = Math.round(cssH * dpr);
  } else {
    canvas.style.width = cssW + 'px';       // still sync CSS (cheap)
    canvas.style.height = cssH + 'px';
  }
  /* Uniform scale into the transform: all drawing stays in logical
   * vW x CR.H units, so projection math is untouched by any resize.
   * When the aspect clamp binds (portrait floor / ultrawide ceiling) the
   * logical box no longer matches the canvas aspect, so the world is
   * scaled UNIFORMLY by the smaller factor and centred — letterboxed
   * bands instead of a stretched road, exactly like an arcade cabinet
   * with a fixed-size screen. */
  if (CRG.ctx) {
    const kx = cssW / vW, ky = cssH / CR.H;
    const k = Math.min(kx, ky);
    CRG.ctx.setTransform(
      k * dpr, 0, 0, k * dpr,
      (cssW - vW * k) * dpr / 2,
      (cssH - CR.H * k) * dpr / 2);
  }
  CRG.cssW = cssW; CRG.cssH = cssH; CRG.dpr = dpr;
}

/* ---- 12. Fullscreen (shared FTFS system) ------------------------------ */
function crFsElement() { return ftFsElement(); }
function crFsSupported() { return ftFsSupported(); }
/** True when the game is in native fullscreen OR the automatic CSS
 *  immersive fallback (both need the height-fitted canvas layout). */
function crIsFullscreen() {
  /* P1.1 · only TRUE fullscreen layouts get full-viewport canvas sizing:
   * the native Fullscreen API, or the explicit immersive fallback class.
   * Plain `ft-playing` (auto-launch inline mode) keeps the normal
   * windowed letterbox — treating it as fullscreen made the canvas
   * overflow its 760px stage cap. */
  return ftIsFullscreen()
    || document.body.classList.contains('ft-immersive');
}
async function crToggleFullscreen() {
  await ftToggleFullscreen('cr', 'cr-fs', '#crFsNotice');
}
function crFsNotice(msg) {
  ftFsNotice('#crFsNotice', msg, 'cr');
}
function crFsReflect() {
  ftFsReflect('cr', 'cr-fs', '#crFsBtn');
}

/* ---- 13. View + lifecycle --------------------------------------------- */
function renderCrView() {
  $('#viewRoot').innerHTML = `<div class="content"><div class="content-inner">
    <div class="ft-game-top">
      <a class="ft-back" href="#funtime" title="Back to the games">← back to games</a>
      <div class="ft-game-title">Cyber Rush</div>
    </div>

    <section class="ft-panel ft-game cr-wide" id="crPanel" aria-label="Cyber Rush street racing game">
      <header class="ft-panel-h">
        <div class="ws-ico ft-ico">${ICONS.funtime}</div>
        <div class="ft-panel-t">
          <div class="ft-name">Soket Street Racing</div>
          <div class="ft-blurb">Neon-night pursuit. Chase the target score before the clock dies — thread the traffic and burn the nitro.</div>
        </div>
      </header>

      <div class="cr-hud" role="status" aria-live="off">
        <span class="si-stat"><span class="si-k">score</span><span class="si-v" id="crScore">0</span></span>
        <span class="si-stat"><span class="si-k">speed</span><span class="si-v" id="crSpeed">0</span></span>
        <span class="si-stat"><span class="si-k">time</span><span class="si-v" id="crTime">0:00</span></span>
        <span class="si-stat"><span class="si-k">dist</span><span class="si-v" id="crDist">0.00 km</span></span>
        <span class="si-stat"><span class="si-k">lap</span><span class="si-v" id="crLap">1</span></span>
        <span class="si-stat"><span class="si-k">passed</span><span class="si-v" id="crPassed">0</span></span>
        <span class="si-stat"><span class="si-k">near miss</span><span class="si-v" id="crNear">0</span></span>
        <span class="si-stat cr-drift" id="crDrift"></span>
        <span class="si-stat cr-combo hidden" id="crCombo"></span>
        <span class="si-stat" id="crModeStat"><span class="si-k" id="crModeK">goal</span><span class="si-v" id="crModeV">0/5000</span></span>
        <span class="si-stat"><span class="si-k">best</span><span class="si-v" id="crBest">0</span></span>
      </div>

      <div class="cr-meter-row" id="crMeterRow">
        <span class="cr-meter-k">nitro</span>
        <span class="cr-meter" aria-hidden="true"><span class="cr-meter-fill" id="crMeterFill"></span></span>
      </div>

      <div class="cr-stage">
        <canvas id="crCanvas" width="800" height="500" aria-label="Racing view"
                role="img">Your browser does not support the canvas element.</canvas>
        <div class="cr-overlay" id="crOverlay" role="dialog" aria-modal="false" aria-label="Game status">
          <!-- PHASE 6 · main menu screen -->
          <div class="cr-ov-inner cr-menu" id="crMenuScreen">
            <div class="cr-ov-title">CYBER RUSH</div>
            <div class="cr-ov-sub">arcade neon racer</div>
            <div class="cr-menu-btns">
              <button type="button" class="ft-btn primary" id="crMQuick">Quick Race</button>
              <button type="button" class="ft-btn" id="crMTime">Time Attack</button>
              <button type="button" class="ft-btn" id="crMEndless">Endless</button>
              <button type="button" class="ft-btn" id="crMChamp">Championship</button>
              <button type="button" class="ft-btn" id="crMGarage">Garage</button>
              <button type="button" class="ft-btn" id="crMRecords">Records</button>
            </div>
            <div class="cr-menu-foot" id="crMenuFoot"></div>
          </div>

          <!-- PHASE 6 · difficulty screen -->
          <div class="cr-ov-inner cr-menu hidden" id="crDiffScreen">
            <div class="cr-ov-title" id="crDiffTitle">Difficulty</div>
            <div class="cr-ov-sub" id="crDiffSub">Pick your challenge</div>
            <div class="cr-menu-btns">
              <button type="button" class="ft-btn" id="crDEasy">Easy</button>
              <button type="button" class="ft-btn primary" id="crDMedium">Medium</button>
              <button type="button" class="ft-btn" id="crDHard">Hard</button>
            </div>
            <div class="cr-diff-note" id="crDiffNote"></div>
            <button type="button" class="ft-btn cr-back" id="crDiffBack">← Back</button>
          </div>

          <!-- PHASE 6 · garage screen -->
          <div class="cr-ov-inner cr-menu cr-garage hidden" id="crGarageScreen">
            <div class="cr-ov-title">GARAGE</div>
            <div class="cr-garage-grid" id="crCarGrid"></div>
            <div class="cr-garage-custom" id="crCustomBox"></div>
            <div class="cr-garage-actions">
              <button type="button" class="ft-btn primary" id="crGarageGo">Race This Car</button>
              <button type="button" class="ft-btn cr-back" id="crGarageBack">← Back</button>
            </div>
          </div>

          <!-- PHASE 6 · records / achievements screen -->
          <div class="cr-ov-inner cr-menu hidden" id="crRecordsScreen">
            <div class="cr-ov-title">RECORDS</div>
            <div class="cr-rec-grid" id="crRecGrid"></div>
            <div class="cr-ach-list" id="crAchList"></div>
            <div class="cr-garage-actions">
              <button type="button" class="ft-btn" id="crResetSave">Reset All Records</button>
              <button type="button" class="ft-btn cr-back" id="crRecordsBack">← Back</button>
            </div>
          </div>

          <!-- PHASE 6 · championship screen -->
          <div class="cr-ov-inner cr-menu hidden" id="crChampScreen">
            <div class="cr-ov-title" id="crChampTitle">CHAMPIONSHIP</div>
            <div class="cr-champ-body" id="crChampBody"></div>
            <div class="cr-garage-actions" id="crChampActions"></div>
          </div>

          <!-- existing ready/pause/over flow -->
          <div class="cr-ov-inner" id="crPlayScreen">
            <div class="cr-ov-title" id="crOvTitle">Cyber Rush</div>
            <div class="cr-ov-sub" id="crOvSub">Steer with ←/→ or A/D</div>
            <div class="cr-ov-stats hidden" id="crOvStats">
              <div class="cr-ov-stat"><span class="k">score</span><span class="v" id="crRsScore">0</span></div>
              <div class="cr-ov-stat"><span class="k">distance</span><span class="v" id="crRsDist">0.00 km</span></div>
              <div class="cr-ov-stat"><span class="k">time</span><span class="v" id="crRsTime">0:00</span></div>
              <div class="cr-ov-stat"><span class="k">max speed</span><span class="v" id="crRsMax">0</span></div>
              <div class="cr-ov-stat"><span class="k">best drift</span><span class="v" id="crRsDrift">0</span></div>
              <div class="cr-ov-stat"><span class="k">near miss</span><span class="v" id="crRsNear">0</span></div>
              <div class="cr-ov-stat"><span class="k">nitro used</span><span class="v" id="crRsNitro">0s</span></div>
              <div class="cr-ov-stat"><span class="k">passed</span><span class="v" id="crRsPassed">0</span></div>
              <div class="cr-ov-stat"><span class="k">crashes</span><span class="v" id="crRsCrashes">0</span></div>
              <div class="cr-ov-stat hidden"><span class="k">laps</span><span class="v" id="crRsLaps">—</span></div>
              <div class="cr-ov-stat hidden"><span class="k">position</span><span class="v" id="crRsPos">—</span></div>
              <div class="cr-ov-stat cr-ov-rating"><span class="k">rating</span><span class="v" id="crRsRating">—</span></div>
            </div>
            <div class="cr-ov-sub hidden" id="crRsBestTag" style="color:var(--acc)">★ new personal best ★</div>
            <div class="cr-ov-progress hidden" id="crOvProgress"></div>
            <button type="button" class="ft-btn primary" id="crOvBtn" data-kind="ready">Start Engine</button>
            <div id="crOvLaunch"></div>
            <div class="ft-actions" role="group" aria-label="Post-race">
              <button type="button" class="ft-btn" id="crOvMenuBtn">Main Menu</button>
            </div>
          </div>
        </div>
      <!-- P1.3 · compact control overlays INSIDE the stage: top-right
           corner chips over the canvas. Semi-transparent, never blocking
           the road (they sit above the skyline area). -->
      <div class="cr-ctl-overlay" role="group" aria-label="Game controls">
        <button type="button" class="cr-chip" id="crPauseBtn" title="Pause (P)">❚❚</button>
        <button type="button" class="cr-chip" id="crRestartBtn" title="Restart (R)">⟳</button>
        <button type="button" class="cr-chip si-sound" id="crSoundBtn" aria-pressed="false" title="Sound">♪</button>
        <button type="button" class="cr-chip" id="crFsBtn" aria-pressed="false" title="Fullscreen (F)">⛶</button>
        <button type="button" class="cr-chip si-exit" id="crExit" title="Back to Games">✕</button>
      </div>
      <span class="cr-snd-lbl" id="crSoundLbl" hidden>sound off</span>

      <!-- P1.3 · touch pads overlaid at the bottom corners of the canvas:
           steer cluster bottom-left, pedals bottom-right. Pointer events
           only on the pads themselves so the road stays fully visible. -->
      <div class="cr-touch cr-touch-ov" role="group" aria-label="Touch controls">
        <button type="button" class="cr-pad" id="crLeft" aria-label="Steer left">◀</button>
        <button type="button" class="cr-pad" id="crRight" aria-label="Steer right">▶</button>
        <button type="button" class="cr-pad" id="crBrake" aria-label="Brake">▼</button>
        <button type="button" class="cr-pad" id="crGas" aria-label="Accelerate">▲</button>
        <button type="button" class="cr-pad nitro" id="crNitro" aria-label="Nitro boost">N₂O</button>
        <button type="button" class="cr-pad drift" id="crDrift" aria-label="Drift">DRIFT</button>
      </div>

        <div class="cr-fs-notice" id="crFsNotice" role="status" aria-live="polite"></div>
      </div>

</section>
  </div></div>`;

  /* Bootstrap. Every listener goes through crOn() so crTeardown() removes
   * them all on route change. */
  CRG.epoch++;
  const myEpoch = CRG.epoch;
  CRG.myEpoch = myEpoch;
  CRG.cleanup = [];
  CRG.reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  const crOn = (target, ev, fn, opts) => {
    target.addEventListener(ev, fn, opts);
    CRG.cleanup.push(() => target.removeEventListener(ev, fn, opts));
  };

  // Fresh state each visit; sound muted by default (consistent with the
  // other games and autoplay policy). PHASE 6: boot into the main menu.
  Object.assign(CRG, crNewGame('quick', 'medium'));
  CRA.on = false;
  CRG.carId = CR_SAVE.car;
  // Traffic must exist before the first frame is drawn.
  CRG.traffic = crSpawnTraffic(CR_DIFFS.medium.traffic);

  crInit($('#crCanvas'));
  crSoundButton();
  crShowMenu();
  crHud();

  // ---- keyboard ----
  const GAME_KEYS = new Set(['ArrowLeft','ArrowRight','ArrowUp','ArrowDown',
    'a','d','w','s','n','A','D','W','S','N',' ','p','r','P','R','f','F','Shift','Escape']);
  crOn(document, 'keydown', e => {
    if (CRG.epoch !== myEpoch) return;
    const t = e.target;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    if (e.key === 'p' || e.key === 'P') { crTogglePause(); e.preventDefault(); return; }
    if (e.key === 'r' || e.key === 'R') { crRestart(); e.preventDefault(); return; }
    if (e.key === 'f' || e.key === 'F') { crToggleFullscreen(); e.preventDefault(); return; }
    // Escape is owned by the shared session controller (ftSessionBind):
    // one press leaves fullscreen AND returns to Fun Time.

    if (GAME_KEYS.has(e.key)) {
      // Guard against OS auto-repeat re-triggering toggles: only a genuine
      // press (no repeat) records state, so held keys stay held but never
      // re-fire side effects.
      CRG.keys[e.key] = true;
      // Never let the game keys scroll the page while playing.
      if (e.key === ' ' || e.key.startsWith('Arrow')) e.preventDefault();
    }
  });
  crOn(window, 'keyup', e => { delete CRG.keys[e.key]; });

  // Held keys must not survive a blur or tab switch.
  crOn(window, 'blur', () => {
    CRG.keys = Object.create(null);
    CRG.touch = { left:false, right:false, gas:false, brake:false, nitro:false, drift:false };
    crPause();
  });
  crOn(document, 'visibilitychange', () => {
    if (document.hidden) {
      CRG.keys = Object.create(null);
      crPause();
    }
  });

  // ---- persistent controls ----
  crOn($('#crPauseBtn'), 'click', () => crTogglePause());
  crOn($('#crRestartBtn'), 'click', () => crRestart());
  crOn($('#crSoundBtn'), 'click', () => { crSoundToggle(); });
  crOn($('#crFsBtn'), 'click', () => { crToggleFullscreen(); });
  if (!crFsSupported()) $('#crFsBtn').style.display = 'none';
  /* FULLSCREEN/VIEWPORT FIX · ONE fullscreenchange listener per mount.
   * The old code registered this transition twice (once here, once in the
   * resize block below) plus a third inside ftSessionBind — three resizes
   * per toggle. A single handler now reflects state AND refits, with a
   * rAF follow-up for engines that apply the fullscreen layout a frame
   * late. crTeardown() removes it with every other crOn binding. */
  const crFsChanged = () => {
    crFsReflect();
    crResize();
    requestAnimationFrame(crResize);
  };
  crOn(document, 'fullscreenchange', crFsChanged);
  crOn(document, 'webkitfullscreenchange', crFsChanged);

  // Shared session lifecycle: automatic fullscreen from the card click,
  // Esc → back to Fun Time, dashboard chrome hidden while playing.
  ftSessionAfterMount('cr', 'cr-fs');
  ftSessionBind(crOn, () => crResize());
  crOn($('#crExit'), 'click', () => { location.hash = '#funtime'; });

  // Overlay button: Start / Resume / Play Again.
  crOn($('#crOvBtn'), 'click', () => {
    const kind = $('#crOvBtn').dataset.kind;
    if (kind === 'over') { crRestart(); crStart(); }
    else crStart();
  });

  /* PHASE 6 · menu flow: mode -> difficulty -> garage -> countdown.
   * The pending mode is remembered while the player picks a difficulty. */
  let pendingMode = 'quick';
  const goDiff = (mode) => { pendingMode = mode; crSfx('ui');
    const t = $('#crDiffTitle');
    if (t) t.textContent = CR_MODES[mode].label;
    const n = $('#crDiffNote');
    if (n) n.textContent = mode === 'endless'
      ? 'Difficulty ramps as you drive — pick your starting point.'
      : mode === 'timeattack'
      ? 'Speed, drifts and near misses extend the clock.'
      : 'Reach the target score to win.';
    crScreen('crDiffScreen'); };
  crOn($('#crMQuick'),  'click', () => goDiff('quick'));
  crOn($('#crMTime'),   'click', () => goDiff('timeattack'));
  crOn($('#crMEndless'),'click', () => goDiff('endless'));
  crOn($('#crMChamp'),  'click', () => { crSfx('ui'); crChampRender(); crScreen('crChampScreen'); });
  crOn($('#crMGarage'), 'click', () => { crSfx('ui'); crGarageRender(); crScreen('crGarageScreen'); });
  crOn($('#crMRecords'),'click', () => { crSfx('ui'); crRecordsRender(); crScreen('crRecordsScreen'); });

  const startDiff = (d) => { crSfx('ui'); crLaunchRace(pendingMode, d, false); };
  crOn($('#crDEasy'),   'click', () => startDiff('easy'));
  crOn($('#crDMedium'), 'click', () => startDiff('medium'));
  crOn($('#crDHard'),   'click', () => startDiff('hard'));
  crOn($('#crDiffBack'),'click', () => { crSfx('ui'); crShowMenu(); });

  crOn($('#crGarageGo'),   'click', () => { crSfx('ui'); crLaunchRace(pendingMode || 'quick', CRG.diff || 'medium', false); });
  crOn($('#crGarageBack'), 'click', () => { crSfx('ui'); crShowMenu(); });
  crOn($('#crRecordsBack'),'click', () => { crSfx('ui'); crShowMenu(); });
  crOn($('#crOvMenuBtn'),  'click', () => { crSfx('ui'); crRestartToMenu(); });
  crOn($('#crResetSave'),  'click', () => {
    if (!confirm('Reset all records, achievements and unlocks?')) return;
    localStorage.removeItem(CR_SAVE_KEY);
    CR_SAVE = crSaveDefault();
    crSaveWrite(); crRecordsRender(); crMenuFootRender(); crSfx('ui');
  });

  // ---- touch pads ----
  const bindHold = (id, key) => {
    const el = $('#' + id);
    const on = e => { e.preventDefault(); CRG.touch[key] = true; };
    const off = e => { e.preventDefault(); CRG.touch[key] = false; };
    crOn(el, 'pointerdown', on);
    crOn(el, 'pointerup', off);
    crOn(el, 'pointercancel', off);
    crOn(el, 'pointerleave', off);
    crOn(el, 'contextmenu', e => e.preventDefault());
  };
  bindHold('crLeft','left'); bindHold('crRight','right');
  bindHold('crGas','gas');   bindHold('crBrake','brake');
  bindHold('crNitro','nitro'); bindHold('crDrift','drift');

  // ---- resize + loop ----
  /* FULLSCREEN/VIEWPORT FIX · resize plumbing, each source registered
   * exactly once (the fullscreenchange path lives above with the state
   * reflection, so there are no duplicate listeners): */
  crOn(window, 'resize', () => crResize());
  /* devicePixelRatio changes (monitor switch, zoom, emulation) fire no
   * resize event. A ResizeObserver on the stage wrapper catches every
   * layout-affecting change exactly once — no polling, and the allocation
   * guard inside crResize makes spurious calls free. This is also what
   * re-fits the canvas when entering/leaving fullscreen changes the stage
   * box, even if the transition events arrive out of order. */
  if (window.ResizeObserver) {
    let roQueued = false;
    const ro = new ResizeObserver(() => {
      /* Coalesce to one resize per frame: the observer can fire several
       * times during a fullscreen transition. */
      if (roQueued) return;
      roQueued = true;
      requestAnimationFrame(() => { roQueued = false; crResize(); });
    });
    const stageEl = document.querySelector('#crPanel .cr-stage');
    if (stageEl) ro.observe(stageEl);
    CRG.cleanup.push(() => ro.disconnect());
  }
  if (window.matchMedia) {
    const mq = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    const onDpr = () => { crResize(); };
    if (mq.addEventListener) { mq.addEventListener('change', onDpr); }
    else if (mq.addListener) { mq.addListener(onDpr); }
    CRG.cleanup.push(() => {
      if (mq.removeEventListener) mq.removeEventListener('change', onDpr);
      else if (mq.removeListener) mq.removeListener(onDpr);
    });
  }

  CRG.lastT = performance.now();
  requestAnimationFrame(crFrame);

  // Launch is automatic from the card click (FTSESS); no manual button.
  $('#crOvLaunch').hidden = true;

  // Draw one static frame immediately so the canvas is never blank.
  crDraw();
}

/** Tear down: cancel the loop, remove listeners, dispose audio, leave
 *  fullscreen. Idempotent — safe to call repeatedly. */
function crTeardown() {
  CRG.epoch++;                     // invalidates the running loop
  if (CRG.raf) { cancelAnimationFrame(CRG.raf); CRG.raf = 0; }
  for (const off of CRG.cleanup) { try { off(); } catch { /* gone */ } }
  CRG.cleanup = [];
  CRG.keys = Object.create(null);
  CRG.touch = { left:false, right:false, gas:false, brake:false, nitro:false, drift:false };
  CRG.ctx = null; CRG.canvas = null;
  CR_STREAKS.length = 0;
  CR_SPARKS.length = 0;
  crFxClear();                                     // Phase 3 pools
  for (const k of Object.keys(CR_HUD_CACHE)) delete CR_HUD_CACHE[k];
  crAudioDispose();
  ftExitFullscreenQuiet();
  document.body.classList.remove('cr-fs', 'si-fs');
  ftSessionRestore();               // restore dashboard chrome
}

/* Diagnostics hook for browser-side verification (mirrors SND/SIG etc).
 * Exposes read-only state snapshots plus safe helpers so tests can drive
 * the game without touching private closures. */
window.CRD = {
  get state() { return CRG.state; },
  get speed() { return Math.round(CRG.speed); },
  get score() { return Math.round(CRG.score); },
  get position() { return Math.round(CRG.position); },
  get playerX() { return +CRG.playerX.toFixed(3); },
  get laps() { return CRG.laps; },
  get passed() { return CRG.passed; },
  get crashes() { return CRG.crashes; },
  get nitroActive() { return !!CRG.nitroActive; },
  get mode() { return CRG.mode; },
  get overlay() { return CRG.overlay; },
  get canvasSize() {
    const c = CRG.canvas;
    return c ? { w: c.width, h: c.height } : null;
  },
  get fps() { return CRD._fps | 0; },
  _fpsAcc: 0, _fpsN: 0, _fps: 0, _fpsT: 0,
  /** Rolling FPS estimate, updated from the frame loop. */
  tick(t) {
    if (!CRD._fpsT) { CRD._fpsT = t; return; }
    const d = t - CRD._fpsT;
    if (d >= 500) {
      CRD._fps = (CRD._fpsN * 1000) / d;
      CRD._fpsN = 0; CRD._fpsT = t;
    }
    CRD._fpsN++;
  },
  start: crStart,
  pause: crPause,
  restart: crRestart,
};


/* Route dispatcher, hardened: a renderer that throws (or a backend that
 * is unreachable) must never leave #viewRoot blank. The existing error
 * banner — the same one every data view already uses on a failed fetch —
 * explains what happened and offers retry, so a connection failure stays
 * visible instead of silently emptying the dashboard. */
async function render() {
  // Welcome is the full-screen launcher: the sidebar is hidden and the
  // four cards use the viewport. Any route with its own view keeps the
  // normal shell. (Unknown routes fall through to the Welcome renderer,
  // hence the inclusive list rather than an exclusion.)
  const WORKSPACE_VIEWS = ['loop', 'chat', 'traces', 'requests', 'models',
    'analytics', 'settings', 'sft', 'pretrain', 'funtime', 'eval',
    'evalbank', 'evaldata', 'evalbench', 'evalmodel', 'evalreport', 'evaltools'];
  document.body.classList.toggle('ws-launcher', !WORKSPACE_VIEWS.includes(S.route));
  // Leaving a pipeline workspace must release its canvas listeners.
  if (S.route !== 'pretrain') paTeardown();
  if (S.route !== 'sft') swTeardown();

  try {
    switch (S.route) {
      // The portal's "Loop AI Console" card routes to the workspace's chat
      // view; the other console views stay reachable from its sidebar.
      case 'loop': return await renderChat();
      case 'chat': return await renderChat();
      case 'traces': return await renderTraces();
      case 'requests': return await renderRequests();
      case 'models': return await renderModels();
      case 'analytics': return await renderAnalytics();
      case 'settings': return await renderSettings();
      case 'sft': return await renderSftPipeline();
      case 'pretrain': return await renderPretrainPipeline();
      case 'funtime': return await renderFunTime();
      // Internal evaluation: ONE unified workspace. #eval renders the
      // Overview; its sections are sub-routes of the same shell
      // (renderers live in eval-workspaces.js, loaded before this
      // script). Legacy top-level hashes (#evalbank …) keep working —
      // they map onto the matching section inside the workspace.
      case 'eval': return await renderEvalWorkspace('overview');
      case 'evalbank': return await renderEvalWorkspace('bank', evSubRoute('evalbank'));
      case 'evaldata': return await renderEvalWorkspace('data');
      case 'evalbench': return await renderEvalWorkspace('bench');
      case 'evalmodel': return await renderEvalWorkspace('model');
      case 'evalreport': return await renderEvalWorkspace('report');
      case 'evaltools': return await renderEvalWorkspace('tools');
      default: return await renderOverview();
    }
  } catch (e) { return errorView(e); }
}

function boot() {
  renderNav();
  refreshHealth();
  const applyHash = () => {
    let h = decodeURIComponent(location.hash.replace(/^#/, '')) || 'overview';
    // Sub-hash views (#funtime/t3, #pretrain/architecture) are still the
    // parent route as far as the shell and nav are concerned; the owning
    // renderer reads the raw hash to decide which sub-view to mount.
    if (h.startsWith('funtime/')) h = 'funtime';
    if (h.startsWith('pretrain/')) h = 'pretrain';
    if (h.startsWith('sft/')) h = 'sft';
    // Unified Internal Evaluation sub-routes: #eval/bank, #eval/data, …
    // map onto the section routes; the raw hash stays intact so the
    // workspace renderer can read its own deep links (#eval/bank/<file>).
    if (h.startsWith('eval/')) {
      const sec = h.slice(5);
      for (const r of ['bank', 'data', 'bench', 'model', 'report', 'tools']) {
        if (sec === r || sec.startsWith(r + '/')) { h = 'eval' + r; break; }
      }
      if (h.startsWith('eval/')) h = 'eval';
    }
    // Legacy top-level eval hashes stay valid as-is.
    if (h !== S.route) S.route = h;
    renderNav();
    render();
  };
  window.addEventListener('hashchange', applyHash);
  applyHash();

  // Bounded health refresh; never overlaps (single timer, visibility-gated).
  setInterval(() => {
    if (document.visibilityState === 'visible') refreshHealth();
  }, 15000);

  // Keyboard: Esc stops generation anywhere in the chat view.
  document.addEventListener('keydown', e => {
    if (e.key === 'Escape' && S.streaming) stopGeneration();
  });
}

/* Boot guard: even if initialization itself fails (unexpected exception
 * before the first render), the user sees the standard error banner with
 * a retry path rather than an empty content area. */
try {
  boot();
} catch (e) {
  try { errorView(e); } catch { /* nothing more we can do */ }
}
initSessionChip();
