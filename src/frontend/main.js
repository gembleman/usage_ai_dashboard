async function loadAll() {
  document.getElementById('status').textContent = 'Loading...';
  try {
    const [usage, accounts, rateLimits] = await Promise.all([
      fetchJson('/api/usage'),
      fetchJson('/api/accounts'),
      fetchJson('/api/rate_limits'),
    ]);
    renderTrendChart(usage);
    renderAccountTable(accounts);
    renderModelChart(usage);
    renderUsageTable(usage);
    renderRateLimits(rateLimits);
    document.getElementById('status').textContent = `Updated ${new Date().toLocaleTimeString()}`;
  } catch (e) {
    document.getElementById('status').textContent = 'Error loading data: ' + e.message;
  }
}

document.getElementById('refreshBtn').addEventListener('click', async () => {
  const btn = document.getElementById('refreshBtn');
  btn.disabled = true;
  document.getElementById('status').textContent = 'Refreshing (re-parsing logs)...';
  try {
    await fetchJson('/api/refresh', { method: 'POST' });
    await loadAll();
  } catch (e) {
    document.getElementById('status').textContent = 'Refresh failed: ' + e.message;
  } finally {
    btn.disabled = false;
  }
});

loadAll();
