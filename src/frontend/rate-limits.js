function meterColor(pct) {
  if (pct >= 90) return 'var(--bad)';
  if (pct >= 70) return 'var(--warn)';
  return 'var(--good)';
}

function renderRateLimits(snapshots) {
  const container = document.getElementById('rateLimits');
  container.innerHTML = '';
  if (snapshots.length === 0) {
    container.innerHTML = '<div class="empty-note">No Codex rate limit snapshots found.</div>';
    return;
  }
  for (const snap of snapshots) {
    const card = document.createElement('div');
    card.className = 'rl-card';
    const observed = new Date(snap.observed_at).toLocaleString();
    let html = `<div class="rl-head">
      <span class="rl-account">${snap.account}${snap.plan_type ? `<span class="tag">${snap.plan_type}</span>` : ''}</span>
      <span class="rl-observed">observed ${observed}</span>
    </div>`;

    const windowHtml = (label, w) => {
      if (!w) return '';
      const pct = Math.min(100, w.used_percent);
      const resets = new Date(w.resets_at * 1000).toLocaleString();
      return `<div class="rl-window">
        <div class="rl-window-label"><span>${label} (${w.window_minutes} min window)</span><span>${w.used_percent.toFixed(1)}% used</span></div>
        <div class="meter"><div class="meter-fill" style="width:${pct}%;background:${meterColor(pct)}"></div></div>
        <div class="rl-window-label"><span></span><span>resets ${resets}</span></div>
      </div>`;
    };

    html += windowHtml('Primary / 5h', snap.primary);
    html += windowHtml('Secondary / 7d', snap.secondary);
    if (snap.rate_limit_reached_type) {
      html += `<div class="rl-window-label" style="color:var(--bad)">rate_limit_reached_type: ${snap.rate_limit_reached_type}</div>`;
    }
    card.innerHTML = html;
    container.appendChild(card);
  }
}
