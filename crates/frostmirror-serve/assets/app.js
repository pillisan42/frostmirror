// frostmirror web UI - minimal shared logic
// Page-specific scripts are inline in the HTML templates.

document.addEventListener('DOMContentLoaded', () => {
    // Auto-refresh dashboard every 30 seconds if on the dashboard page
    if (window.location.pathname === '/') {
        setInterval(() => {
            fetch('/api/status')
                .then(r => r.json())
                .then(d => {
                    const el = (id) => document.getElementById(id);
                    if (el('crate-count')) el('crate-count').textContent = d.crate_count;
                    if (el('mirror-size')) el('mirror-size').textContent = d.total_size_human;
                    if (el('last-import')) el('last-import').textContent = d.last_import || 'never';
                    if (el('watcher-state')) el('watcher-state').textContent = d.watcher_active ? 'active' : 'disabled';
                    if (el('done-count')) el('done-count').textContent = d.done_count;
                    if (el('failed-count')) el('failed-count').textContent = d.failed_count;
                })
                .catch(() => {});
        }, 30000);
    }
});
