const PALETTE = ['#7c9eff', '#4fd1a5', '#f5c26b', '#f47174', '#b98cf0', '#5bc8e0', '#e08fd0', '#9fd15a'];
const fmt = n => n.toLocaleString('en-US');
const tooltip = document.getElementById('tooltip');

function showTooltip(evt, html) {
  tooltip.innerHTML = html;
  tooltip.style.display = 'block';
  tooltip.style.left = (evt.clientX + 14) + 'px';
  tooltip.style.top = (evt.clientY + 14) + 'px';
}
function hideTooltip() { tooltip.style.display = 'none'; }

function seriesKey(row) {
  return `${row.source}/${row.account}`;
}

function colorFor(key, keys) {
  const idx = keys.indexOf(key);
  return PALETTE[idx % PALETTE.length];
}

async function fetchJson(url, opts) {
  const res = await fetch(url, opts);
  if (!res.ok) throw new Error(`${url} -> ${res.status}`);
  return res.json();
}
