const ACCOUNT_RANGE_DAYS = { '1d': 1, '7d': 7, '30d': 30, '365d': 365, all: null };
let accountTableState = { usageRows: [], range: 'all' };

// 기준 날짜(가장 최근 데이터 날짜)로부터 range일 이내의 레코드만 남긴다.
function filterUsageRowsByRange(usageRows, range) {
  const days = ACCOUNT_RANGE_DAYS[range];
  if (!days || usageRows.length === 0) return usageRows;
  const latest = usageRows.reduce((max, r) => (r.date > max ? r.date : max), usageRows[0].date);
  const cutoff = new Date(latest + 'T00:00:00Z');
  cutoff.setUTCDate(cutoff.getUTCDate() - (days - 1));
  const cutoffStr = cutoff.toISOString().slice(0, 10);
  return usageRows.filter(r => r.date >= cutoffStr);
}

// 상세(usage) 데이터를 (source, account) 기준으로 집계해 계정별 합계 행을 만든다.
function aggregateAccountRows(usageRows) {
  const groups = new Map();
  for (const r of usageRows) {
    const key = `${r.source}/${r.account}`;
    let g = groups.get(key);
    if (!g) {
      g = { source: r.source, account: r.account, input_tokens: 0, cached_input_tokens: 0, output_tokens: 0, total_tokens: 0, turns: 0, cost: 0, hasCost: false };
      groups.set(key, g);
    }
    g.input_tokens += r.input_tokens;
    g.cached_input_tokens += r.cached_input_tokens;
    g.output_tokens += r.output_tokens;
    g.total_tokens += r.total_tokens;
    g.turns += r.turns;
    const c = estimateCostUsd(r.model, r.input_tokens, r.cached_input_tokens, r.output_tokens);
    if (c != null) { g.cost += c; g.hasCost = true; }
  }
  return [...groups.values()].sort((a, b) => a.source.localeCompare(b.source) || a.account.localeCompare(b.account));
}

function renderAccountTable(usageRows) {
  accountTableState.usageRows = usageRows || [];
  applyAccountRangeFilter();
}

function setAccountRange(range) {
  accountTableState.range = range;
  applyAccountRangeFilter();

  document.querySelectorAll('#accountRangeTabs .tab-btn').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.range === range);
  });
}

function applyAccountRangeFilter() {
  const filtered = filterUsageRowsByRange(accountTableState.usageRows, accountTableState.range);
  const rows = aggregateAccountRows(filtered);

  const tbody = document.querySelector('#accountTable tbody');
  tbody.innerHTML = '';
  if (rows.length === 0) {
    tbody.innerHTML = '<tr><td colspan="8" class="empty-note">데이터가 없습니다.</td></tr>';
    return;
  }
  for (const r of rows) {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td>${r.source}</td><td>${r.account}</td>` +
      `<td title="${fmt(r.input_tokens)}">${fmtKo(r.input_tokens)}</td><td title="${fmt(r.cached_input_tokens)}">${fmtKo(r.cached_input_tokens)}</td>` +
      `<td title="${fmt(r.output_tokens)}">${fmtKo(r.output_tokens)}</td><td title="${fmt(r.total_tokens)}">${fmtKo(r.total_tokens)}</td>` +
      `<td>${fmt(r.turns)}</td><td>${fmtUsd(r.hasCost ? r.cost : null)}</td>`;
    tbody.appendChild(tr);
  }
}

const USAGE_PAGE_SIZE = 50;
let usageTableState = { raw: [], sorted: [], page: 1, source: 'all', account: 'all' };

// (source, date) 기준으로 완전 합산. 계정이 'all'이 아니면 해당 계정만 대상으로 한다.
// 비용은 모델별 단가가 다르므로 모델 단위로 먼저 계산한 뒤 합산한다.
function aggregateUsageRows(rows) {
  const groups = new Map();
  for (const r of rows) {
    const key = `${r.source}/${r.date}`;
    const cost = estimateCostUsd(r.model, r.input_tokens, r.cached_input_tokens, r.output_tokens);
    let g = groups.get(key);
    if (!g) {
      g = { source: r.source, date: r.date, input_tokens: 0, cached_input_tokens: 0, output_tokens: 0, total_tokens: 0, turns: 0, cost: 0, hasCost: false };
      groups.set(key, g);
    }
    g.input_tokens += r.input_tokens;
    g.cached_input_tokens += r.cached_input_tokens;
    g.output_tokens += r.output_tokens;
    g.total_tokens += r.total_tokens;
    g.turns += r.turns;
    if (cost != null) { g.cost += cost; g.hasCost = true; }
  }
  return [...groups.values()].sort((a, b) => (a.date < b.date ? 1 : a.date > b.date ? -1 : 0));
}

function updateAccountOptions(rows) {
  const select = document.getElementById('accountSelect');
  const accounts = [...new Set(rows.map(r => r.account))].sort();
  const current = select.value || 'all';
  select.innerHTML = '<option value="all">전체 (합산)</option>' +
    accounts.map(a => `<option value="${a}">${a}</option>`).join('');
  select.value = accounts.includes(current) ? current : 'all';
}

function renderUsageTable(rows) {
  usageTableState.raw = rows;
  updateAccountOptions(rows);
  applyUsageTableFilters();
}

function filterBySource(rows, source) {
  return source === 'all' ? rows : rows.filter(r => r.source === source);
}

function filterByAccount(rows, account) {
  return account === 'all' ? rows : rows.filter(r => r.account === account);
}

function applyUsageTableFilters() {
  const { raw, source, account } = usageTableState;
  const filtered = filterByAccount(filterBySource(raw, source), account);
  usageTableState.sorted = aggregateUsageRows(filtered);
  usageTableState.page = 1;
  renderUsageTablePage();
}

function setUsageTableSource(source) {
  usageTableState.source = source;
  applyUsageTableFilters();

  document.querySelectorAll('#sourceTabs .tab-btn').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.source === source);
  });
}

function setUsageTableAccount(account) {
  usageTableState.account = account;
  applyUsageTableFilters();
}

function renderUsageTablePage() {
  const { sorted, page } = usageTableState;
  const tbody = document.querySelector('#usageTable tbody');
  const pagination = document.getElementById('usagePagination');
  tbody.innerHTML = '';
  pagination.innerHTML = '';

  if (sorted.length === 0) {
    tbody.innerHTML = '<tr><td colspan="9" class="empty-note">데이터가 없습니다.</td></tr>';
    return;
  }

  const totalPages = Math.max(1, Math.ceil(sorted.length / USAGE_PAGE_SIZE));
  const clampedPage = Math.min(Math.max(1, page), totalPages);
  usageTableState.page = clampedPage;

  const start = (clampedPage - 1) * USAGE_PAGE_SIZE;
  const pageRows = sorted.slice(start, start + USAGE_PAGE_SIZE);
  const accountLabel = usageTableState.account === 'all' ? '전체' : usageTableState.account;

  for (const r of pageRows) {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td>${r.source}</td><td>${accountLabel}</td><td>${r.date}</td>` +
      `<td title="${fmt(r.input_tokens)}">${fmtKo(r.input_tokens)}</td><td title="${fmt(r.cached_input_tokens)}">${fmtKo(r.cached_input_tokens)}</td>` +
      `<td title="${fmt(r.output_tokens)}">${fmtKo(r.output_tokens)}</td><td title="${fmt(r.total_tokens)}">${fmtKo(r.total_tokens)}</td>` +
      `<td>${fmt(r.turns)}</td><td>${fmtUsd(r.hasCost ? r.cost : null)}</td>`;
    tbody.appendChild(tr);
  }

  renderUsagePagination(totalPages, clampedPage);
}

function renderUsagePagination(totalPages, page) {
  const pagination = document.getElementById('usagePagination');
  if (totalPages <= 1) return;

  const goTo = p => {
    usageTableState.page = p;
    renderUsageTablePage();
  };

  const addBtn = (label, targetPage, opts = {}) => {
    const btn = document.createElement('button');
    btn.textContent = label;
    btn.className = 'page-btn';
    if (opts.active) btn.classList.add('active');
    btn.disabled = !!opts.disabled;
    btn.addEventListener('click', () => goTo(targetPage));
    pagination.appendChild(btn);
  };

  addBtn('이전', page - 1, { disabled: page === 1 });

  const windowSize = 2;
  for (let p = 1; p <= totalPages; p++) {
    if (p === 1 || p === totalPages || Math.abs(p - page) <= windowSize) {
      addBtn(String(p), p, { active: p === page });
    } else if (Math.abs(p - page) === windowSize + 1) {
      const dots = document.createElement('span');
      dots.className = 'page-ellipsis';
      dots.textContent = '...';
      pagination.appendChild(dots);
    }
  }

  addBtn('다음', page + 1, { disabled: page === totalPages });

  const info = document.createElement('span');
  info.className = 'page-info';
  info.textContent = `${page} / ${totalPages} 페이지`;
  pagination.appendChild(info);
}
