function renderAccountTable(rows) {
  const tbody = document.querySelector('#accountTable tbody');
  tbody.innerHTML = '';
  if (rows.length === 0) {
    tbody.innerHTML = '<tr><td colspan="7" class="empty-note">No data.</td></tr>';
    return;
  }
  for (const r of rows) {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td>${r.source}</td><td>${r.account}</td>` +
      `<td>${fmt(r.input_tokens)}</td><td>${fmt(r.cached_input_tokens)}</td>` +
      `<td>${fmt(r.output_tokens)}</td><td>${fmt(r.total_tokens)}</td><td>${fmt(r.turns)}</td>`;
    tbody.appendChild(tr);
  }
}

function renderUsageTable(rows) {
  const tbody = document.querySelector('#usageTable tbody');
  tbody.innerHTML = '';
  if (rows.length === 0) {
    tbody.innerHTML = '<tr><td colspan="9" class="empty-note">No data.</td></tr>';
    return;
  }
  const sorted = [...rows].sort((a, b) => (a.date < b.date ? 1 : a.date > b.date ? -1 : 0));
  for (const r of sorted) {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td>${r.source}</td><td>${r.account}</td><td>${r.date}</td><td>${r.model}</td>` +
      `<td>${fmt(r.input_tokens)}</td><td>${fmt(r.cached_input_tokens)}</td>` +
      `<td>${fmt(r.output_tokens)}</td><td>${fmt(r.total_tokens)}</td><td>${fmt(r.turns)}</td>`;
    tbody.appendChild(tr);
  }
}
