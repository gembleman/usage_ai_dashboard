import { emptyNote, emptyTableRow, fetchJson, fmtTime } from './util.js';
import { renderRateLimits } from './rate-limits.js';
import {
  renderGlobalFilteredPanels,
  setGlobalRange,
  setUsageTableAccount,
  setUsageTableSource,
} from './tables.js';

// 지정된 컨테이너 id에 에러 메시지를 표시한다 (해당 패널이 의존하는 API가 실패한 경우).
function showPanelError(containerId, message, colspan) {
  const el = document.getElementById(containerId);
  if (!el) return;
  if (el.tagName === 'TABLE') {
    const tbody = el.querySelector('tbody') || el;
    tbody.replaceChildren(colspan ? emptyTableRow(colspan, message) : emptyNote(message));
  } else {
    el.replaceChildren(emptyNote(message));
  }
}

function showUsagePanelErrors(message) {
  showPanelError('trendChart', message);
  document.getElementById('trendLegend').replaceChildren();
  showPanelError('accountTable', message, 8);
  showPanelError('modelChart', message);
  document.getElementById('modelLegend').replaceChildren();
  showPanelError('usageTable', message, 10);
  document.getElementById('usagePagination').replaceChildren();
}

async function loadAll() {
  document.getElementById('status').textContent = '불러오는 중...';

  const [usageResult, rateLimitsResult] = await Promise.allSettled([
    fetchJson('/api/usage'),
    fetchJson('/api/rate_limits'),
  ]);

  if (usageResult.status === 'fulfilled') {
    const usage = usageResult.value;
    // renderGlobalFilteredPanels가 전역 기간 필터 파이프라인을 통해 상세 내역 테이블까지
    // 함께 그리므로 renderUsageTable을 따로 호출하지 않는다(중복/덮어쓰기 방지).
    renderGlobalFilteredPanels(usage);
  } else {
    showUsagePanelErrors('사용량 데이터를 불러오지 못했습니다: ' + usageResult.reason.message);
  }

  if (rateLimitsResult.status === 'fulfilled') {
    renderRateLimits(rateLimitsResult.value);
  } else {
    showPanelError('rateLimits', '요청 한도 데이터를 불러오지 못했습니다: ' + rateLimitsResult.reason.message);
  }

  const failures = [usageResult, rateLimitsResult].filter(r => r.status === 'rejected');
  if (failures.length === 0) {
    document.getElementById('status').textContent = `${fmtTime(Date.now())} 기준 업데이트됨`;
  } else if (failures.length === 2) {
    document.getElementById('status').textContent = '데이터를 불러오지 못했습니다.';
  } else {
    document.getElementById('status').textContent = `일부 데이터를 불러오지 못했습니다 (${fmtTime(Date.now())} 기준 부분 업데이트).`;
  }
}

document.getElementById('refreshBtn').addEventListener('click', async () => {
  const btn = document.getElementById('refreshBtn');
  btn.disabled = true;
  document.getElementById('status').textContent = '새로고침 중 (로그 재분석)...';
  try {
    await fetchJson('/api/refresh', { method: 'POST', timeoutMs: 120000 });
    await loadAll();
  } catch (e) {
    document.getElementById('status').textContent = '새로고침 실패: ' + e.message;
  } finally {
    btn.disabled = false;
  }
});

document.getElementById('sourceTabs').addEventListener('click', (e) => {
  const btn = e.target.closest('.tab-btn');
  if (!btn) return;
  setUsageTableSource(btn.dataset.source);
});

document.getElementById('accountSelect').addEventListener('change', (e) => {
  setUsageTableAccount(e.target.value);
});

document.getElementById('globalRangeTabs').addEventListener('click', (e) => {
  const btn = e.target.closest('.tab-btn');
  if (!btn) return;
  setGlobalRange(btn.dataset.range);
});

loadAll();
