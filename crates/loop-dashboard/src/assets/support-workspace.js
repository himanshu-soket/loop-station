'use strict';
/* ============================================================
   HELP & SUPPORT — header-button workspace
   Opened from the topbar button (#supportBtn), never a portal card.

   One button, two experiences, decided by the SERVER:
     · normal user  → "Raise a Request"  (submit + track own tickets)
     · administrator → "Support Management" (all tickets, replies,
                        status changes, private internal notes)

   The role probe (/api/support/me) only picks UI. Every endpoint under
   /api/support/* re-resolves the caller's role server-side, so a user
   who somehow loads admin markup still receives 403/404 from the API.
   ============================================================ */

/* Support surface state. Kept separate from the console's S so a failing
 * support fetch can never disturb the rest of the dashboard. */
const SUP = {
  scope: null,            // {isAdmin, identifier, userId, openTickets}
  mode: 'form',           // 'form' | 'mine' | 'admin' | 'detail'
  mine: { items: [], total: 0, page: 1, perPage: 25, status: 'all' },
  admin: {
    items: [], total: 0, page: 1, perPage: 25,
    q: '', status: 'all', category: 'all', priority: 'all',
    summary: null,
  },
  detail: null,           // {ticket, attachments, events, canManage}
  detailId: null,
};

/* ---------- vocabulary (mirrors the backend constants exactly) ---------- */
const SUP_CATEGORIES = [
  ['technical_issue', 'Technical Issue'],
  ['bug_report', 'Bug Report'],
  ['login_problem', 'Login Problem'],
  ['performance_issue', 'Performance Issue'],
  ['data_pipeline', 'Data Pipeline'],
  ['model_evaluation', 'Model Evaluation'],
  ['feature_request', 'Feature Request'],
  ['general_inquiry', 'General Inquiry'],
  ['other', 'Other'],
];
const SUP_PRIORITIES = [
  ['low', 'Low'], ['medium', 'Medium'], ['high', 'High'], ['critical', 'Critical'],
];
const SUP_STATUSES = [
  ['pending', 'Pending'],
  ['in_progress', 'In Progress'],
  ['waiting_user', 'Waiting for User'],
  ['resolved', 'Resolved'],
  ['closed', 'Closed'],
];

const supLabel = (list, key) => {
  const hit = list.find(([k]) => k === key);
  return hit ? hit[1] : key;
};
const supCatLabel = k => supLabel(SUP_CATEGORIES, k);
const supPriLabel = k => supLabel(SUP_PRIORITIES, k);
const supStatusLabel = k => supLabel(SUP_STATUSES, k);

/* ---------- small presentational helpers ---------- */
function supPill(kind, value) {
  return `<span class="pill ${kind}">${esc(value)}</span>`;
}
function supStatusPill(status) {
  const kind = status === 'resolved' || status === 'closed' ? 'ok'
    : status === 'pending' ? 'warn'
    : status === 'waiting_user' ? 'inc' : 'ok';
  return supPill(kind, supStatusLabel(status));
}
function supPriorityPill(priority) {
  const kind = priority === 'critical' ? 'bad'
    : priority === 'high' ? 'warn' : 'inc';
  return supPill(kind, supPriLabel(priority));
}

/* ---------- header button wiring ---------- */

/* Ask the server which experience this caller gets, then reveal the button.
 * Any failure leaves it hidden — never a false admin badge. */
async function supInitHeaderButton() {
  const btn = document.getElementById('supportBtn');
  if (!btn) return;
  try {
    const scope = await api('/api/support/me');
    SUP.scope = scope;
    const label = document.getElementById('supportBtnLabel');
    if (label) {
      label.textContent = scope.isAdmin ? 'Support Management' : 'Raise a Request';
    }
    btn.title = scope.isAdmin
      ? 'Review and manage support requests'
      : 'Submit a support request';
    btn.hidden = false;
    btn.onclick = supOpen;
    // Admins get a live pending-count badge (open tickets).
    supRefreshBadge();
  } catch { /* unreachable or unauthenticated: keep hidden */ }
}

/* Refresh just the badge count without touching the rest of the UI. */
async function supRefreshBadge() {
  const badge = document.getElementById('supportBadge');
  if (!badge || !SUP.scope) return;
  if (!SUP.scope.isAdmin) { badge.hidden = true; return; }
  try {
    const scope = await api('/api/support/me');
    const n = Number(scope.openTickets) || 0;
    if (n > 0) {
      badge.textContent = n > 99 ? '99+' : String(n);
      badge.hidden = false;
    } else {
      badge.hidden = true;
    }
  } catch { badge.hidden = true; }
}

/* ---------- overlay shell ----------
 * A fixed overlay above the current view (the dashboard keeps its state
 * underneath). Esc or a backdrop click closes it. */

function supOverlayNode() {
  let node = document.getElementById('supportOverlay');
  if (node) return node;
  node = el('div', 'sup-overlay');
  node.id = 'supportOverlay';
  node.innerHTML = '<div class="sup-panel" role="dialog" aria-modal="true" ' +
    'aria-label="Help and Support" id="supportPanel"></div>';
  document.body.appendChild(node);
  node.addEventListener('mousedown', e => {
    // Close only on a direct backdrop click, not a bubbled child event.
    if (e.target === node) supClose();
  });
  return node;
}

let supEscHandler = null;

function supOpen() {
  const node = supOverlayNode();
  node.classList.add('open');
  document.body.classList.add('sup-open'); // lock scroll behind the overlay
  supEscHandler = e => { if (e.key === 'Escape') supClose(); };
  document.addEventListener('keydown', supEscHandler);
  // Default landing: users see the submission form, admins the dashboard.
  supRender(SUP.scope && SUP.scope.isAdmin ? 'admin' : 'form');
}

function supClose() {
  const node = document.getElementById('supportOverlay');
  if (node) node.classList.remove('open');
  document.body.classList.remove('sup-open');
  if (supEscHandler) {
    document.removeEventListener('keydown', supEscHandler);
    supEscHandler = null;
  }
  // Drop detail state so reopening starts fresh.
  SUP.detail = null;
  SUP.detailId = null;
}

/* ---------- render dispatcher ---------- */

function supRender(mode) {
  SUP.mode = mode;
  const panel = document.getElementById('supportPanel');
  if (!panel) return;
  if (mode === 'form') return supRenderForm(panel);
  if (mode === 'mine') return supRenderMine(panel);
  if (mode === 'admin') return supRenderAdmin(panel);
  if (mode === 'detail') return supRenderDetail(panel);
}

/* ---------- view: submission form (normal user) ---------- */

function supRenderForm(panel) {
  const me = SUP.scope || {};
  panel.innerHTML = `
    <div class="sup-head">
      <div>
        <div class="sup-title">Raise a Request</div>
        <div class="sup-sub">Describe the issue and we'll get back to you at your account email.</div>
      </div>
      <button type="button" class="sup-close" id="supCloseBtn" title="Close">✕</button>
    </div>
    <div class="sup-body">
      <div class="sup-tabs">
        <button type="button" class="sup-tab active" data-mode="form">New request</button>
        <button type="button" class="sup-tab" data-mode="mine">My requests</button>
      </div>
      <form id="supForm" novalidate>
        <div class="sup-grid">
          <div class="field">
            <label for="supName">Full name</label>
            <input id="supName" type="text" value="${esc(me.identifier || '')}" readonly disabled>
          </div>
          <div class="field">
            <label for="supEmail">Email address</label>
            <input id="supEmail" type="email" value="${esc(me.identifier || '')}" readonly disabled>
          </div>
          <div class="field">
            <label for="supSubject">Subject</label>
            <input id="supSubject" type="text" maxlength="200" placeholder="Short summary of the issue" required>
          </div>
          <div class="field">
            <label for="supCategory">Category</label>
            <select id="supCategory">
              ${SUP_CATEGORIES.map(([v, l]) => `<option value="${v}">${l}</option>`).join('')}
            </select>
          </div>
          <div class="field">
            <label for="supPriority">Priority</label>
            <select id="supPriority">
              ${SUP_PRIORITIES.map(([v, l]) =>
                `<option value="${v}"${v === 'medium' ? ' selected' : ''}>${l}</option>`).join('')}
            </select>
          </div>
        </div>
        <div class="field">
          <label for="supDescription">Detailed description</label>
          <textarea id="supDescription" rows="6" maxlength="20000"
            placeholder="What happened? What did you expect? Include any steps to reproduce."></textarea>
        </div>
        <div class="field">
          <label for="supFiles">Screenshot / attachment</label>
          <input id="supFiles" type="file" multiple
            accept="image/png,image/jpeg,image/gif,image/webp,image/svg+xml,application/pdf,text/plain,application/json,text/csv,application/zip,text/markdown">
          <div class="field-hint">Up to 5 files · 5 MiB each · images, PDF, text, CSV, JSON or ZIP.</div>
          <div class="sup-files" id="supFileList"></div>
        </div>
        <div class="alert alert-error sup-alert" id="supError" hidden></div>
        <div class="sup-actions">
          <button type="submit" class="btn btn-primary sup-submit" id="supSubmit">Submit request</button>
        </div>
      </form>
    </div>`;

  document.getElementById('supCloseBtn').onclick = supClose;
  panel.querySelectorAll('.sup-tab').forEach(t => {
    t.onclick = () => supRender(t.dataset.mode);
  });

  const filesInput = document.getElementById('supFiles');
  let picked = [];
  const drawFiles = () => {
    const box = document.getElementById('supFileList');
    box.innerHTML = picked.map((f, i) => `
      <div class="sup-file">
        <span class="sup-file-name" title="${esc(f.name)}">${esc(f.name)}</span>
        <span class="sup-file-meta">${(f.size / 1024).toFixed(1)} KB</span>
        <button type="button" class="sup-file-x" data-i="${i}" title="Remove">✕</button>
      </div>`).join('');
    box.querySelectorAll('.sup-file-x').forEach(b => {
      b.onclick = () => {
        picked.splice(Number(b.dataset.i), 1);
        drawFiles();
      };
    });
  };
  filesInput.onchange = () => {
    picked = Array.from(filesInput.files || []).slice(0, 5);
    drawFiles();
  };

  document.getElementById('supForm').onsubmit = async e => {
    e.preventDefault();
    await supSubmitForm(picked);
  };
}

async function supSubmitForm(picked) {
  const errBox = document.getElementById('supError');
  const btn = document.getElementById('supSubmit');
  errBox.hidden = true;
  const subject = document.getElementById('supSubject').value.trim();
  const category = document.getElementById('supCategory').value;
  const priority = document.getElementById('supPriority').value;
  const description = document.getElementById('supDescription').value.trim();

  // Client-side validation mirrors the server's rules; the server remains
  // authoritative and re-validates everything.
  if (!subject) return supShowError('Please add a subject.');
  if (!description) return supShowError('Please describe the issue.');

  // Read + encode attachments client-side. Size/type limits are enforced
  // again server-side; this early check gives instant feedback.
  const attachments = [];
  for (const f of picked) {
    if (f.size > 5 * 1024 * 1024) {
      return supShowError(`${f.name} is larger than 5 MiB.`);
    }
    try {
      const b64 = await supFileToBase64(f);
      attachments.push({ filename: f.name, contentType: f.type || 'application/octet-stream', data: b64 });
    } catch {
      return supShowError(`Could not read ${f.name}.`);
    }
  }

  btn.disabled = true;
  btn.textContent = 'Submitting…';
  try {
    const res = await api('/api/support/tickets', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ subject, category, priority, description, attachments }),
    });
    supRenderSuccess(res.ticket, res.emailStatus);
    supRefreshBadge();
  } catch (err) {
    supShowError(err.message || 'Submission failed; please try again.');
    btn.disabled = false;
    btn.textContent = 'Submit request';
  }
}

function supShowError(message) {
  const box = document.getElementById('supError');
  if (!box) return;
  box.innerHTML = `<span class="alert-icon">!</span><span>${esc(message)}</span>`;
  box.hidden = false;
}

function supFileToBase64(file) {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => {
      // Strip the data-URL prefix ("data:<mime>;base64,").
      const s = String(r.result || '');
      const i = s.indexOf(',');
      resolve(i >= 0 ? s.slice(i + 1) : s);
    };
    r.onerror = () => reject(new Error('read failed'));
    r.readAsDataURL(file);
  });
}

/* Success confirmation with the generated ticket ID. */
function supRenderSuccess(ticket, emailStatus) {
  const panel = document.getElementById('supportPanel');
  const emailNote = emailStatus && emailStatus !== 'sent'
    ? `<div class="sup-note">${esc(emailStatus)}</div>` : '';
  panel.innerHTML = `
    <div class="sup-head">
      <div>
        <div class="sup-title">Request submitted</div>
        <div class="sup-sub">We received your request and will reply at your account email.</div>
      </div>
      <button type="button" class="sup-close" id="supCloseBtn" title="Close">✕</button>
    </div>
    <div class="sup-body sup-success">
      <div class="sup-mark">✓</div>
      <div class="sup-ticket-id">${esc(ticket.ticketId)}</div>
      <div class="sup-success-sub">Keep this ticket ID handy — quote it in any follow-up.</div>
      <div class="sup-summary">
        <div class="sup-summary-row"><span>Subject</span><strong>${esc(ticket.subject)}</strong></div>
        <div class="sup-summary-row"><span>Category</span><strong>${esc(supCatLabel(ticket.category))}</strong></div>
        <div class="sup-summary-row"><span>Priority</span><strong>${esc(supPriLabel(ticket.priority))}</strong></div>
        <div class="sup-summary-row"><span>Status</span><strong>${esc(supStatusLabel(ticket.status))}</strong></div>
      </div>
      ${emailNote}
      <div class="sup-actions">
        <button type="button" class="btn btn-primary" id="supViewMine">Track my requests</button>
        <button type="button" class="sup-btn-ghost" id="supAnother">Raise another</button>
      </div>
    </div>`;
  document.getElementById('supCloseBtn').onclick = supClose;
  document.getElementById('supViewMine').onclick = () => supRender('mine');
  document.getElementById('supAnother').onclick = () => supRender('form');
}

/* ---------- view: my requests (normal user) ---------- */

async function supRenderMine(panel) {
  panel.innerHTML = `
    <div class="sup-head">
      <div>
        <div class="sup-title">My Requests</div>
        <div class="sup-sub">Requests you have submitted, newest first.</div>
      </div>
      <button type="button" class="sup-close" id="supCloseBtn" title="Close">✕</button>
    </div>
    <div class="sup-body">
      <div class="sup-tabs">
        <button type="button" class="sup-tab" data-mode="form">New request</button>
        <button type="button" class="sup-tab active" data-mode="mine">My requests</button>
      </div>
      <div class="sup-filters">
        <select id="supMineStatus" class="sup-select">
          <option value="all">All statuses</option>
          ${SUP_STATUSES.map(([v, l]) => `<option value="${v}">${l}</option>`).join('')}
        </select>
      </div>
      <div class="sup-list" id="supMineList"><div class="sup-loading">loading…</div></div>
      <div class="sup-foot" id="supMineFoot"></div>
    </div>`;
  document.getElementById('supCloseBtn').onclick = supClose;
  panel.querySelectorAll('.sup-tab').forEach(t => {
    t.onclick = () => supRender(t.dataset.mode);
  });
  const statusSel = document.getElementById('supMineStatus');
  statusSel.value = SUP.mine.status;
  statusSel.onchange = () => {
    SUP.mine.status = statusSel.value;
    SUP.mine.page = 1;
    supLoadMine();
  };
  await supLoadMine();
}

async function supLoadMine() {
  const box = document.getElementById('supMineList');
  if (!box) return;
  const params = new URLSearchParams({
    page: String(SUP.mine.page),
    perPage: String(SUP.mine.perPage),
  });
  if (SUP.mine.status !== 'all') params.set('status', SUP.mine.status);
  try {
    const res = await api('/api/support/tickets?' + params.toString());
    SUP.mine.items = res.items || [];
    SUP.mine.total = res.total || 0;
    supDrawMine();
  } catch (err) {
    box.innerHTML = `<div class="sup-empty">${esc(err.message || 'Could not load requests.')}</div>`;
  }
}

function supDrawMine() {
  const box = document.getElementById('supMineList');
  const foot = document.getElementById('supMineFoot');
  if (!box) return;
  if (!SUP.mine.items.length) {
    box.innerHTML = `
      <div class="sup-empty">
        <div class="sup-empty-t">No requests yet</div>
        <div class="sup-empty-s">When you submit a request it appears here with its live status.</div>
      </div>`;
    if (foot) foot.innerHTML = '';
    return;
  }
  box.innerHTML = SUP.mine.items.map(t => `
    <div class="sup-row" data-id="${esc(t.ticketId)}" tabindex="0" role="button">
      <div class="sup-row-main">
        <div class="sup-row-top">
          <span class="mono sup-row-id">${esc(t.ticketId)}</span>
          ${supStatusPill(t.status)}
          ${supPriorityPill(t.priority)}
        </div>
        <div class="sup-row-subject">${esc(t.subject)}</div>
        <div class="sup-row-meta dim">${esc(supCatLabel(t.category))} · ${fmtTime(t.createdAtMs)}</div>
      </div>
      <span class="sup-chevron">›</span>
    </div>`).join('');
  box.querySelectorAll('.sup-row').forEach(r => {
    const open = () => supOpenDetail(r.dataset.id);
    r.onclick = open;
    r.onkeydown = e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); open(); } };
  });
  if (foot) {
    const pages = Math.max(1, Math.ceil(SUP.mine.total / SUP.mine.perPage));
    foot.innerHTML = `
      <span class="mono faint">${SUP.mine.total} request${SUP.mine.total === 1 ? '' : 's'}</span>
      <div class="sup-pager">
        <button type="button" class="sup-page" id="supMinePrev" ${SUP.mine.page <= 1 ? 'disabled' : ''}>‹ prev</button>
        <span class="mono faint">${SUP.mine.page} / ${pages}</span>
        <button type="button" class="sup-page" id="supMineNext" ${SUP.mine.page >= pages ? 'disabled' : ''}>next ›</button>
      </div>`;
    const prev = document.getElementById('supMinePrev');
    const next = document.getElementById('supMineNext');
    if (prev) prev.onclick = () => { SUP.mine.page--; supLoadMine(); };
    if (next) next.onclick = () => { SUP.mine.page++; supLoadMine(); };
  }
}

/* ---------- view: admin dashboard ---------- */

async function supRenderAdmin(panel) {
  panel.innerHTML = `
    <div class="sup-head">
      <div>
        <div class="sup-title">Support Management</div>
        <div class="sup-sub">Every user-submitted request, with replies, status and history.</div>
      </div>
      <button type="button" class="sup-close" id="supCloseBtn" title="Close">✕</button>
    </div>
    <div class="sup-body">
      <div class="sup-cards" id="supCards"><div class="sup-loading">loading…</div></div>
      <div class="sup-filters">
        <input type="search" id="supAdminQ" class="sup-search" placeholder="Search ticket ID, subject or email…"
          value="${esc(SUP.admin.q)}">
        <select id="supAdminStatus" class="sup-select">
          <option value="all">All statuses</option>
          ${SUP_STATUSES.map(([v, l]) =>
            `<option value="${v}"${SUP.admin.status === v ? ' selected' : ''}>${l}</option>`).join('')}
        </select>
        <select id="supAdminCategory" class="sup-select">
          <option value="all">All categories</option>
          ${SUP_CATEGORIES.map(([v, l]) =>
            `<option value="${v}"${SUP.admin.category === v ? ' selected' : ''}>${l}</option>`).join('')}
        </select>
        <select id="supAdminPriority" class="sup-select">
          <option value="all">All priorities</option>
          ${SUP_PRIORITIES.map(([v, l]) =>
            `<option value="${v}"${SUP.admin.priority === v ? ' selected' : ''}>${l}</option>`).join('')}
        </select>
      </div>
      <div class="sup-table-wrap">
        <table class="sup-table">
          <thead><tr>
            <th>Ticket ID</th><th>User</th><th>Email</th><th>Subject</th>
            <th>Category</th><th>Priority</th><th>Status</th><th>Submitted</th><th></th>
          </tr></thead>
          <tbody id="supAdminRows"><tr><td colspan="9" class="sup-loading">loading…</td></tr></tbody>
        </table>
      </div>
      <div class="sup-foot" id="supAdminFoot"></div>
    </div>`;
  document.getElementById('supCloseBtn').onclick = supClose;

  const q = document.getElementById('supAdminQ');
  q.oninput = debounce(() => {
    SUP.admin.q = q.value.trim();
    SUP.admin.page = 1;
    supLoadAdmin();
  }, 300);
  for (const [id, key] of [
    ['supAdminStatus', 'status'],
    ['supAdminCategory', 'category'],
    ['supAdminPriority', 'priority'],
  ]) {
    const sel = document.getElementById(id);
    sel.onchange = () => {
      SUP.admin[key] = sel.value;
      SUP.admin.page = 1;
      supLoadAdmin();
    };
  }
  await Promise.all([supLoadAdminSummary(), supLoadAdmin()]);
}

async function supLoadAdminSummary() {
  const box = document.getElementById('supCards');
  if (!box) return;
  try {
    const s = await api('/api/support/admin/summary');
    SUP.admin.summary = s;
    const cards = [
      ['Total Tickets', s.total, ''],
      ['Pending Requests', s.pending, 'warn'],
      ['In Progress', s.inProgress, ''],
      ['Resolved', s.resolved, 'ok'],
      ['Critical Issues', s.critical, 'bad'],
    ];
    box.innerHTML = cards.map(([label, n, kind]) => `
      <div class="sup-card ${kind}">
        <div class="sup-card-n">${fmtInt(n)}</div>
        <div class="sup-card-l">${esc(label)}</div>
      </div>`).join('');
  } catch { box.innerHTML = ''; }
}

async function supLoadAdmin() {
  const body = document.getElementById('supAdminRows');
  if (!body) return;
  const params = new URLSearchParams({
    page: String(SUP.admin.page),
    perPage: String(SUP.admin.perPage),
  });
  if (SUP.admin.q) params.set('q', SUP.admin.q);
  if (SUP.admin.status !== 'all') params.set('status', SUP.admin.status);
  if (SUP.admin.category !== 'all') params.set('category', SUP.admin.category);
  if (SUP.admin.priority !== 'all') params.set('priority', SUP.admin.priority);
  try {
    const res = await api('/api/support/admin/tickets?' + params.toString());
    SUP.admin.items = res.items || [];
    SUP.admin.total = res.total || 0;
    supDrawAdminRows();
  } catch (err) {
    // A 403 here means the role probe was stale (e.g. demoted mid-session):
    // fall back to the user experience rather than showing a broken table.
    if (String(err.message || '').includes('administrator')) {
      SUP.scope = await api('/api/support/me');
      supRender(SUP.scope.isAdmin ? 'admin' : 'form');
      return;
    }
    body.innerHTML = `<tr><td colspan="9" class="sup-empty">${esc(err.message || 'Could not load tickets.')}</td></tr>`;
  }
}

function supDrawAdminRows() {
  const body = document.getElementById('supAdminRows');
  const foot = document.getElementById('supAdminFoot');
  if (!body) return;
  if (!SUP.admin.items.length) {
    body.innerHTML = `
      <tr><td colspan="9">
        <div class="sup-empty">
          <div class="sup-empty-t">No tickets match</div>
          <div class="sup-empty-s">Adjust the filters or clear the search to see more.</div>
        </div>
      </td></tr>`;
    if (foot) foot.innerHTML = '';
    return;
  }
  body.innerHTML = SUP.admin.items.map(t => `
    <tr class="sup-tr" data-id="${esc(t.ticketId)}">
      <td class="mono sup-cell-id">${esc(t.ticketId)}</td>
      <td class="sup-cell-user" title="${esc(t.userName)}">${esc(t.userName)}</td>
      <td class="sup-cell-email mono" title="${esc(t.userEmail)}">${esc(t.userEmail)}</td>
      <td class="sup-cell-subject" title="${esc(t.subject)}">${esc(t.subject)}</td>
      <td>${esc(supCatLabel(t.category))}</td>
      <td>${supPriorityPill(t.priority)}</td>
      <td>${supStatusPill(t.status)}</td>
      <td class="dim sup-cell-date" title="${fmtTimeFull(t.createdAtMs)}">${fmtTime(t.createdAtMs)}</td>
      <td><button type="button" class="sup-open-btn" data-id="${esc(t.ticketId)}">Open</button></td>
    </tr>`).join('');
  body.querySelectorAll('.sup-tr, .sup-open-btn').forEach(node => {
    const handler = e => {
      e.stopPropagation();
      supOpenDetail(node.dataset.id);
    };
    if (node.tagName === 'TR') {
      node.onclick = handler;
      node.tabIndex = 0;
      node.onkeydown = e => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); supOpenDetail(node.dataset.id); }
      };
    } else {
      node.onclick = handler;
    }
  });
  if (foot) {
    const pages = Math.max(1, Math.ceil(SUP.admin.total / SUP.admin.perPage));
    foot.innerHTML = `
      <span class="mono faint">${SUP.admin.total} ticket${SUP.admin.total === 1 ? '' : 's'}</span>
      <div class="sup-pager">
        <button type="button" class="sup-page" id="supAdminPrev" ${SUP.admin.page <= 1 ? 'disabled' : ''}>‹ prev</button>
        <span class="mono faint">${SUP.admin.page} / ${pages}</span>
        <button type="button" class="sup-page" id="supAdminNext" ${SUP.admin.page >= pages ? 'disabled' : ''}>next ›</button>
      </div>`;
    const prev = document.getElementById('supAdminPrev');
    const next = document.getElementById('supAdminNext');
    if (prev) prev.onclick = () => { SUP.admin.page--; supLoadAdmin(); };
    if (next) next.onclick = () => { SUP.admin.page++; supLoadAdmin(); };
  }
}

/* ---------- view: ticket detail (user + admin) ---------- */

async function supOpenDetail(ticketId) {
  SUP.detailId = ticketId;
  const panel = document.getElementById('supportPanel');
  if (!panel) return;
  panel.innerHTML = `<div class="sup-body"><div class="sup-loading">loading…</div></div>`;
  try {
    const d = await api('/api/support/tickets/' + encodeURIComponent(ticketId));
    SUP.detail = d;
    supRenderDetail(panel);
  } catch (err) {
    panel.innerHTML = `
      <div class="sup-head">
        <div><div class="sup-title">Ticket</div></div>
        <button type="button" class="sup-close" id="supCloseBtn" title="Close">✕</button>
      </div>
      <div class="sup-body"><div class="sup-empty">${esc(err.message || 'Could not load the ticket.')}</div></div>`;
    document.getElementById('supCloseBtn').onclick = supClose;
  }
}

function supRenderDetail(panel) {
  const d = SUP.detail;
  if (!d) return;
  const t = d.ticket;
  const isAdmin = !!d.canManage;
  const backMode = isAdmin ? 'admin' : 'mine';
  const events = (d.events || []).map(ev => {
    const label = ev.kind === 'created' ? 'Created'
      : ev.kind === 'status_changed' ? `Status → ${supStatusLabel(ev.body)}`
      : ev.kind === 'reply' ? 'Reply'
      : 'Internal note';
    const who = ev.authorEmail
      ? `${esc(ev.authorEmail)}${ev.authorIsAdmin ? ' · admin' : ''}`
      : 'system';
    const bodyText = ev.kind === 'status_changed' ? '' : ev.body;
    return `
      <div class="sup-ev ${ev.kind}">
        <div class="sup-ev-head">
          <span class="sup-ev-kind">${esc(label)}</span>
          <span class="sup-ev-who">${who}</span>
          <span class="sup-ev-at mono faint">${fmtTimeFull(ev.atMs)}</span>
        </div>
        ${bodyText ? `<div class="sup-ev-body">${mdToHtml(bodyText)}</div>` : ''}
      </div>`;
  }).join('');

  const attachments = (d.attachments || []).length
    ? d.attachments.map(a => `
        <a class="sup-att" href="/api/support/attachments/${a.id}" target="_blank" rel="noopener">
          <span class="sup-att-name" title="${esc(a.filename)}">${esc(a.filename)}</span>
          <span class="sup-att-meta mono">${(a.sizeBytes / 1024).toFixed(1)} KB · ${esc(a.contentType)}</span>
        </a>`).join('')
    : '<span class="faint" style="font-size:12px">none</span>';

  const adminControls = isAdmin ? `
    <div class="sup-manage">
      <div class="sup-manage-row">
        <label class="filter-label" for="supStatusSel">Status</label>
        <select id="supStatusSel" class="sup-select">
          ${SUP_STATUSES.map(([v, l]) =>
            `<option value="${v}"${v === t.status ? ' selected' : ''}>${l}</option>`).join('')}
        </select>
        <button type="button" class="sup-btn-ok" id="supApplyStatus">Update status</button>
      </div>
      <div class="sup-manage-block">
        <label class="filter-label" for="supReplyText">Reply to user</label>
        <textarea id="supReplyText" rows="4" maxlength="20000"
          placeholder="Your reply is emailed to the user and shown in their request view."></textarea>
        <button type="button" class="sup-btn-ok" id="supSendReply">Send reply</button>
      </div>
      <div class="sup-manage-block">
        <label class="filter-label" for="supNoteText">Private internal note</label>
        <textarea id="supNoteText" rows="3" maxlength="20000"
          placeholder="Visible to administrators only — never shown to the user."></textarea>
        <button type="button" class="sup-btn-ghost" id="supAddNote">Add note</button>
      </div>
      <div class="alert alert-error sup-alert" id="supManageError" hidden></div>
    </div>` : '';

  panel.innerHTML = `
    <div class="sup-head">
      <div>
        <div class="sup-title mono">${esc(t.ticketId)}</div>
        <div class="sup-sub">${esc(t.subject)}</div>
      </div>
      <button type="button" class="sup-close" id="supCloseBtn" title="Close">✕</button>
    </div>
    <div class="sup-body">
      <button type="button" class="sup-back" id="supBack">‹ Back</button>
      <div class="sup-detail-grid">
        <div class="sup-detail-main">
          <div class="sup-section">
            <div class="sup-section-t">Issue description</div>
            <div class="sup-desc">${mdToHtml(t.description)}</div>
          </div>
          <div class="sup-section">
            <div class="sup-section-t">Attachments</div>
            <div class="sup-atts">${attachments}</div>
          </div>
          <div class="sup-section">
            <div class="sup-section-t">History</div>
            <div class="sup-events">${events || '<span class="faint" style="font-size:12px">no events</span>'}</div>
          </div>
        </div>
        <aside class="sup-detail-side">
          <div class="sup-side-card">
            <div class="sup-side-row"><span>Status</span>${supStatusPill(t.status)}</div>
            <div class="sup-side-row"><span>Priority</span>${supPriorityPill(t.priority)}</div>
            <div class="sup-side-row"><span>Category</span><strong>${esc(supCatLabel(t.category))}</strong></div>
            <div class="sup-side-row"><span>Submitted</span><strong>${fmtTimeFull(t.createdAtMs)}</strong></div>
            <div class="sup-side-row"><span>Updated</span><strong>${fmtTimeFull(t.updatedAtMs)}</strong></div>
          </div>
          <div class="sup-side-card">
            <div class="sup-side-row"><span>User</span><strong>${esc(t.userName)}</strong></div>
            <div class="sup-side-row"><span>Email</span><strong class="sup-break">${esc(t.userEmail)}</strong></div>
          </div>
          ${adminControls}
        </aside>
      </div>
    </div>`;

  document.getElementById('supCloseBtn').onclick = supClose;
  document.getElementById('supBack').onclick = () => supRender(backMode);

  if (isAdmin) {
    const err = document.getElementById('supManageError');
    const showErr = m => {
      err.innerHTML = `<span class="alert-icon">!</span><span>${esc(m)}</span>`;
      err.hidden = false;
    };
    document.getElementById('supApplyStatus').onclick = async () => {
      const sel = document.getElementById('supStatusSel');
      err.hidden = true;
      try {
        await api(`/api/support/admin/tickets/${encodeURIComponent(t.ticketId)}/status`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ status: sel.value, expectedCurrent: t.status }),
        });
        toast('Status updated');
        await supOpenDetail(t.ticketId);
        supRefreshBadge();
        supLoadAdminSummary();
      } catch (e) { showErr(e.message || 'Could not update the status.'); }
    };
    document.getElementById('supSendReply').onclick = async () => {
      const ta = document.getElementById('supReplyText');
      err.hidden = true;
      const message = ta.value.trim();
      if (!message) return showErr('Write a reply first.');
      try {
        await api(`/api/support/admin/tickets/${encodeURIComponent(t.ticketId)}/reply`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ message }),
        });
        toast('Reply sent');
        await supOpenDetail(t.ticketId);
      } catch (e) { showErr(e.message || 'Could not send the reply.'); }
    };
    document.getElementById('supAddNote').onclick = async () => {
      const ta = document.getElementById('supNoteText');
      err.hidden = true;
      const note = ta.value.trim();
      if (!note) return showErr('Write a note first.');
      try {
        await api(`/api/support/admin/tickets/${encodeURIComponent(t.ticketId)}/note`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ note }),
        });
        toast('Note added');
        await supOpenDetail(t.ticketId);
      } catch (e) { showErr(e.message || 'Could not add the note.'); }
    };
  }
}
