import {
  SOURCE_LABELS,
  emptyTableRow,
  estimateCostUsd,
  fmt,
  fmtKo,
  fmtUsd,
  updateWithViewTransition,
} from './util.js';
import { renderModelChart, renderTrendChart, setModelChartMaxItems } from './charts.js';

const ACCOUNT_RANGE_DAYS = { '1d': 1, '7d': 7, '30d': 30, '365d': 365, all: null };

// 대시보드 전역 기간 필터 상태. 트렌드 차트/계정별 합계/모델별 분포가 모두 이 값을 참조한다.
let globalRangeState = { range: 'all', rawRows: [] };

// 기준 날짜(가장 최근 데이터 날짜)로부터 range일 이내의 레코드만 남긴다.
function filterUsageRowsByRange(usageRows, range) {
  const days = ACCOUNT_RANGE_DAYS[range];
  if (!days || usageRows.length === 0) return usageRows;
  const latest = usageRows.reduce((max, r) => (r.date > max ? r.date : max), usageRows[0].date);
  const cutoffStr = Temporal.PlainDate.from(latest).subtract({ days: days - 1 }).toString();
  return usageRows.filter(r => r.date >= cutoffStr);
}

// 원본 usage 데이터를 저장하고, 현재 선택된 전역 기간으로 필터링해
// 트렌드 차트 / 계정별 합계 / 모델별 분포를 다시 그린다.
export function renderGlobalFilteredPanels(usageRows) {
  globalRangeState.rawRows = usageRows || [];
  applyGlobalRangeFilter();
}

export function setGlobalRange(range) {
  globalRangeState.range = range;
  updateWithViewTransition(applyGlobalRangeFilter);

  document.querySelectorAll('#globalRangeTabs .tab-btn').forEach(btn => {
    const active = btn.dataset.range === range;
    btn.classList.toggle('active', active);
    btn.setAttribute('aria-selected', String(active));
  });
}

function applyGlobalRangeFilter() {
  const ranged = filterUsageRowsByRange(globalRangeState.rawRows, globalRangeState.range);
  // <synthetic>은 Claude Code가 토큰 사용 없는 턴(API 에러 등)에 남기는
  // 플레이스홀더 모델명이라 모든 패널에서 제외한다. 이전에는 상세 테이블만
  // 제외해 계정별 합계 turns가 불일치하는 문제가 있었다.
  const filtered = ranged.filter(r => r.model !== '<synthetic>');
  renderAccountTable(filtered);
  renderTrendChart(filtered);
  renderModelChart(filtered);
  // 상세 내역 테이블도 전역 기간 필터를 따른다. renderUsageTable은 raw만 교체하고
  // 기존 source/account 탭 선택은 usageTableState/updateAccountOptions가 보존한다
  // (기간 변경으로 데이터셋이 바뀌므로 page 리셋은 자연스럽다).
  renderUsageTable(filtered);
}

// 상세(usage) 데이터를 (source, account) 기준으로 집계해 계정별 합계 행을 만든다.
function aggregateAccountRows(usageRows) {
  return [...Map.groupBy(usageRows, r => `${r.source}/${r.account}`).values()]
    .map(group => {
      const first = group[0];
      const g = { source: first.source, account: first.account, input_tokens: 0, cached_input_tokens: 0, cache_creation_input_tokens: 0, output_tokens: 0, total_tokens: 0, turns: 0, cost: 0, hasCost: false };
      for (const r of group) {
        g.input_tokens += r.input_tokens;
        g.cached_input_tokens += r.cached_input_tokens;
        g.cache_creation_input_tokens += (r.cache_creation_input_tokens || 0);
        g.output_tokens += r.output_tokens;
        g.total_tokens += r.total_tokens;
        g.turns += r.turns;
        const c = estimateCostUsd(r.model, r.input_tokens, r.cached_input_tokens, r.cache_creation_input_tokens, r.output_tokens);
        if (c != null) { g.cost += c; g.hasCost = true; }
      }
      return g;
    })
    .sort((a, b) => a.source.localeCompare(b.source) || a.account.localeCompare(b.account));
}

function sumUsageGroup(group) {
  const first = group[0];
  const g = { source: first.source, account: first.account, date: first.date, model: first.model, input_tokens: 0, cached_input_tokens: 0, cache_creation_input_tokens: 0, output_tokens: 0, total_tokens: 0, turns: 0, cost: 0, hasCost: false };
  for (const r of group) {
    g.input_tokens += r.input_tokens;
    g.cached_input_tokens += r.cached_input_tokens;
    g.cache_creation_input_tokens += (r.cache_creation_input_tokens || 0);
    g.output_tokens += r.output_tokens;
    g.total_tokens += r.total_tokens;
    g.turns += r.turns;
    const c = estimateCostUsd(r.model, r.input_tokens, r.cached_input_tokens, r.cache_creation_input_tokens, r.output_tokens);
    if (c != null) { g.cost += c; g.hasCost = true; }
  }
  return g;
}

function appendTextCell(tr, text, title) {
  const td = document.createElement('td');
  td.textContent = text;
  if (title) td.title = title;
  tr.appendChild(td);
}

function appendAccountRow(parent, r) {
  const tr = document.createElement('tr');
  appendTextCell(tr, SOURCE_LABELS[r.source] || r.source);
  appendTextCell(tr, r.account);
  appendTextCell(tr, fmtKo(r.input_tokens), fmt(r.input_tokens));
  appendTextCell(tr, fmtKo(r.cached_input_tokens), fmt(r.cached_input_tokens));
  appendTextCell(tr, fmtKo(r.output_tokens), fmt(r.output_tokens));
  appendTextCell(tr, fmtKo(r.total_tokens), fmt(r.total_tokens));
  appendTextCell(tr, fmt(r.turns));
  appendTextCell(tr, fmtUsd(r.hasCost ? r.cost : null));
  parent.appendChild(tr);
}

function renderAccountTable(usageRows) {
  const rows = aggregateAccountRows(usageRows || []);
  const tbody = document.querySelector('#accountTable tbody');

  if (rows.length === 0) {
    tbody.replaceChildren(emptyTableRow(8, '데이터가 없습니다.'));
    return;
  }

  const fragment = document.createDocumentFragment();
  for (const r of rows) appendAccountRow(fragment, r);
  tbody.replaceChildren(fragment);
}

let usagePageSize = 50;

export function applyDashboardSettings(settings) {
  if (Number.isInteger(settings?.page_size) && settings.page_size > 0) usagePageSize = settings.page_size;
  setModelChartMaxItems(settings?.model_chart_max_items);
}
let usageTableState = { raw: [], sorted: [], page: 1, source: 'all', account: 'all' };

// (source, account, date, model) 기준으로 합산. 계정이 'all'이 아니면 해당 계정만 대상으로 한다.
// 비용은 모델별 단가가 다르므로 모델 단위로 먼저 계산한 뒤 합산한다.
function aggregateUsageRows(rows) {
  // 날짜 내림차순을 우선하되, 같은 날짜 내에서는 source/account/model 순으로 안정 정렬한다.
  return [...Map.groupBy(rows, r => `${r.source}/${r.account}/${r.date}/${r.model}`).values()]
    .map(sumUsageGroup)
    .sort((a, b) =>
    (a.date < b.date ? 1 : a.date > b.date ? -1 : 0) ||
    a.source.localeCompare(b.source) ||
    a.account.localeCompare(b.account) ||
    a.model.localeCompare(b.model)
  );
}

// 선택된 소스(source)에 해당하는 계정만 옵션으로 보여준다. 현재 선택된 계정이
// 새 목록에 없으면(예: 소스 탭 전환) "전체"로 리셋한다.
function updateAccountOptions(rows, source) {
  const select = document.getElementById('accountSelect');
  const scoped = filterBySource(rows, source);
  const accounts = [...new Set(scoped.map(r => r.account))].sort();
  const current = select.value || 'all';
  select.replaceChildren(
    new Option('전체 (합산)', 'all'),
    ...accounts.map(a => new Option(a, a))
  );
  const next = accounts.includes(current) ? current : 'all';
  select.value = next;
  return next;
}

export function renderUsageTable(rows) {
  usageTableState.raw = rows;
  const account = updateAccountOptions(rows, usageTableState.source);
  usageTableState.account = account;
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

export function setUsageTableSource(source) {
  usageTableState.source = source;
  // 소스가 바뀌면 계정 목록도 해당 소스 기준으로 갱신하고, 기존 선택 계정이
  // 새 목록에 없으면 "전체"로 되돌린다.
  updateWithViewTransition(() => {
    usageTableState.account = updateAccountOptions(usageTableState.raw, source);
    applyUsageTableFilters();
  });

  document.querySelectorAll('#sourceTabs .tab-btn').forEach(btn => {
    const active = btn.dataset.source === source;
    btn.classList.toggle('active', active);
    btn.setAttribute('aria-selected', String(active));
  });
}

export function setUsageTableAccount(account) {
  usageTableState.account = account;
  updateWithViewTransition(applyUsageTableFilters);
}

function appendUsageRow(parent, r) {
  const tr = document.createElement('tr');
  appendTextCell(tr, SOURCE_LABELS[r.source] || r.source);
  appendTextCell(tr, r.account);
  appendTextCell(tr, r.date);
  appendTextCell(tr, r.model);
  appendTextCell(tr, fmtKo(r.input_tokens), fmt(r.input_tokens));
  appendTextCell(tr, fmtKo(r.cached_input_tokens), fmt(r.cached_input_tokens));
  appendTextCell(tr, fmtKo(r.output_tokens), fmt(r.output_tokens));
  appendTextCell(tr, fmtKo(r.total_tokens), fmt(r.total_tokens));
  appendTextCell(tr, fmt(r.turns));
  appendTextCell(tr, fmtUsd(r.hasCost ? r.cost : null));
  parent.appendChild(tr);
}

function renderUsageTablePage() {
  const { sorted, page } = usageTableState;
  const tbody = document.querySelector('#usageTable tbody');
  const pagination = document.getElementById('usagePagination');
  pagination.replaceChildren();

  if (sorted.length === 0) {
    tbody.replaceChildren(emptyTableRow(10, '데이터가 없습니다.'));
    return;
  }

  const totalPages = Math.max(1, Math.ceil(sorted.length / usagePageSize));
  const clampedPage = Math.min(Math.max(1, page), totalPages);
  usageTableState.page = clampedPage;

  const start = (clampedPage - 1) * usagePageSize;
  const pageRows = sorted.slice(start, start + usagePageSize);

  const fragment = document.createDocumentFragment();
  for (const r of pageRows) appendUsageRow(fragment, r);
  tbody.replaceChildren(fragment);

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
