function renderAccountTable(rows, usageRows) {
  const tbody = document.querySelector('#accountTable tbody');
  tbody.innerHTML = '';
  if (rows.length === 0) {
    tbody.innerHTML = '<tr><td colspan="8" class="empty-note">데이터가 없습니다.</td></tr>';
    return;
  }
  // 계정 합계에는 모델 정보가 없으므로, 상세(usage) 데이터로 계정별 예상 비용을 재집계한다.
  const costByAccount = {};
  for (const u of usageRows || []) {
    const key = `${u.source}/${u.account}`;
    const c = estimateCostUsd(u.model, u.input_tokens, u.cached_input_tokens, u.output_tokens);
    if (c != null) costByAccount[key] = (costByAccount[key] || 0) + c;
  }
  for (const r of rows) {
    const tr = document.createElement('tr');
    const cost = costByAccount[`${r.source}/${r.account}`];
    tr.innerHTML = `<td>${r.source}</td><td>${r.account}</td>` +
      `<td title="${fmt(r.input_tokens)}">${fmtKo(r.input_tokens)}</td><td title="${fmt(r.cached_input_tokens)}">${fmtKo(r.cached_input_tokens)}</td>` +
      `<td title="${fmt(r.output_tokens)}">${fmtKo(r.output_tokens)}</td><td title="${fmt(r.total_tokens)}">${fmtKo(r.total_tokens)}</td>` +
      `<td>${fmt(r.turns)}</td><td>${fmtUsd(cost)}</td>`;
    tbody.appendChild(tr);
  }
}

const USAGE_PAGE_SIZE = 50;
let usageTableState = { sorted: [], page: 1 };

function renderUsageTable(rows) {
  const sorted = [...rows].sort((a, b) => (a.date < b.date ? 1 : a.date > b.date ? -1 : 0));
  usageTableState = { sorted, page: 1 };
  renderUsageTablePage();
}

function renderUsageTablePage() {
  const { sorted, page } = usageTableState;
  const tbody = document.querySelector('#usageTable tbody');
  const pagination = document.getElementById('usagePagination');
  tbody.innerHTML = '';
  pagination.innerHTML = '';

  if (sorted.length === 0) {
    tbody.innerHTML = '<tr><td colspan="10" class="empty-note">데이터가 없습니다.</td></tr>';
    return;
  }

  const totalPages = Math.max(1, Math.ceil(sorted.length / USAGE_PAGE_SIZE));
  const clampedPage = Math.min(Math.max(1, page), totalPages);
  usageTableState.page = clampedPage;

  const start = (clampedPage - 1) * USAGE_PAGE_SIZE;
  const pageRows = sorted.slice(start, start + USAGE_PAGE_SIZE);

  for (const r of pageRows) {
    const tr = document.createElement('tr');
    const cost = estimateCostUsd(r.model, r.input_tokens, r.cached_input_tokens, r.output_tokens);
    tr.innerHTML = `<td>${r.source}</td><td>${r.account}</td><td>${r.date}</td><td>${r.model}</td>` +
      `<td title="${fmt(r.input_tokens)}">${fmtKo(r.input_tokens)}</td><td title="${fmt(r.cached_input_tokens)}">${fmtKo(r.cached_input_tokens)}</td>` +
      `<td title="${fmt(r.output_tokens)}">${fmtKo(r.output_tokens)}</td><td title="${fmt(r.total_tokens)}">${fmtKo(r.total_tokens)}</td>` +
      `<td>${fmt(r.turns)}</td><td>${fmtUsd(cost)}</td>`;
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
