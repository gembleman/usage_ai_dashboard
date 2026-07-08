async function loadAll() {
  document.getElementById('status').textContent = '불러오는 중...';
  try {
    const [usage, accounts, rateLimits] = await Promise.all([
      fetchJson('/api/usage'),
      fetchJson('/api/accounts'),
      fetchJson('/api/rate_limits'),
    ]);
    renderTrendChart(usage);
    renderAccountTable(accounts, usage);
    renderModelChart(usage);
    renderUsageTable(usage);
    renderRateLimits(rateLimits);
    document.getElementById('status').textContent = `${new Date().toLocaleTimeString('ko-KR')} 기준 업데이트됨`;
  } catch (e) {
    document.getElementById('status').textContent = '데이터를 불러오지 못했습니다: ' + e.message;
  }
}

document.getElementById('refreshBtn').addEventListener('click', async () => {
  const btn = document.getElementById('refreshBtn');
  btn.disabled = true;
  document.getElementById('status').textContent = '새로고침 중 (로그 재분석)...';
  try {
    await fetchJson('/api/refresh', { method: 'POST' });
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

loadAll();
