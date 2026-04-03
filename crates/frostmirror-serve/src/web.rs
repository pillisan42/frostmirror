use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/deps", get(deps_page))
        .route("/config", get(config_page))
        .route("/packages", get(packages_page))
        .route("/setup", get(setup_page))
        .route("/assets/style.css", get(stylesheet))
        .route("/assets/app.js", get(script))
}

fn page_wrapper(title: &str, nav_active: &str, content: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>{title} - frostmirror</title>
    <link rel="stylesheet" href="/assets/style.css">
</head>
<body>
    <nav>
        <div class="brand">frostmirror</div>
        <a href="/" class="{dash_cls}">Dashboard</a>
        <a href="/deps" class="{deps_cls}">Dependencies</a>
        <a href="/config" class="{conf_cls}">Configuration</a>
        <a href="/packages" class="{pkgs_cls}">Packages</a>
        <a href="/setup" class="{setup_cls}">Client Setup</a>
    </nav>
    <main>{content}</main>
    <script src="/assets/app.js"></script>
</body>
</html>"#,
        title = title,
        content = content,
        dash_cls = if nav_active == "dashboard" { "active" } else { "" },
        deps_cls = if nav_active == "deps" { "active" } else { "" },
        conf_cls = if nav_active == "config" { "active" } else { "" },
        pkgs_cls = if nav_active == "packages" { "active" } else { "" },
        setup_cls = if nav_active == "setup" { "active" } else { "" },
    )
}

async fn dashboard() -> Html<String> {
    Html(page_wrapper("Dashboard", "dashboard", r#"
        <h1>Dashboard</h1>
        <div class="grid" id="status-grid">
            <div class="card">
                <h3>Crate Count</h3>
                <p class="big-number" id="crate-count">-</p>
            </div>
            <div class="card">
                <h3>Mirror Size</h3>
                <p class="big-number" id="mirror-size">-</p>
            </div>
            <div class="card">
                <h3>Last Import</h3>
                <p id="last-import">-</p>
            </div>
            <div class="card">
                <h3>Watcher</h3>
                <p id="watcher-state">-</p>
            </div>
            <div class="card">
                <h3>Imported</h3>
                <p class="big-number" id="done-count">-</p>
            </div>
            <div class="card alert">
                <h3>Failed</h3>
                <p class="big-number" id="failed-count">-</p>
            </div>
        </div>
        <script>
        fetch('/api/status').then(r=>r.json()).then(d=>{
            document.getElementById('crate-count').textContent=d.crate_count;
            document.getElementById('mirror-size').textContent=d.total_size_human;
            document.getElementById('last-import').textContent=d.last_import||'never';
            document.getElementById('watcher-state').textContent=d.watcher_active?'active':'disabled';
            document.getElementById('done-count').textContent=d.done_count;
            document.getElementById('failed-count').textContent=d.failed_count;
            if(d.failed_count>0)document.querySelector('.card.alert').classList.add('has-failures');
        });
        </script>
    "#))
}

async fn deps_page() -> Html<String> {
    Html(page_wrapper("Dependencies", "deps", r#"
        <h1>Dependencies</h1>
        <div class="two-col">
            <div>
                <table id="deps-table">
                    <thead><tr><th>Crate</th><th>Version</th><th></th></tr></thead>
                    <tbody></tbody>
                </table>
                <button onclick="addRow()">+ Add Dependency</button>
                <button class="primary" onclick="saveDeps()">Save</button>
            </div>
            <div>
                <h3>TOML Preview</h3>
                <pre id="toml-preview"></pre>
            </div>
        </div>
        <script>
        let deps={};
        fetch('/api/deps').then(r=>r.json()).then(d=>{
            deps=d;
            renderDeps();
        });
        function renderDeps(){
            const tb=document.querySelector('#deps-table tbody');
            tb.innerHTML='';
            for(const[name,ver]of Object.entries(deps.dependencies||{})){
                const tr=document.createElement('tr');
                tr.innerHTML=`<td><input value="${name}" onchange="updateDeps()"></td><td><input value="${ver}" onchange="updateDeps()"></td><td><button onclick="this.closest('tr').remove();updateDeps()">x</button></td>`;
                tb.appendChild(tr);
            }
            updatePreview();
        }
        function addRow(){
            const tb=document.querySelector('#deps-table tbody');
            const tr=document.createElement('tr');
            tr.innerHTML='<td><input placeholder="crate name" onchange="updateDeps()"></td><td><input placeholder="version" onchange="updateDeps()"></td><td><button onclick="this.closest(\'tr\').remove();updateDeps()">x</button></td>';
            tb.appendChild(tr);
        }
        function updateDeps(){
            const rows=document.querySelectorAll('#deps-table tbody tr');
            deps.dependencies={};
            rows.forEach(r=>{
                const inputs=r.querySelectorAll('input');
                if(inputs[0].value)deps.dependencies[inputs[0].value]=inputs[1].value;
            });
            updatePreview();
        }
        function updatePreview(){
            let toml='[dependencies]\n';
            for(const[n,v]of Object.entries(deps.dependencies||{}))toml+=`${n} = "${v}"\n`;
            document.getElementById('toml-preview').textContent=toml;
        }
        function saveDeps(){
            fetch('/api/deps',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(deps)})
            .then(r=>r.json()).then(d=>{alert(d.ok?'Saved!':'Error: '+d.error)});
        }
        </script>
    "#))
}

async fn config_page() -> Html<String> {
    Html(page_wrapper("Configuration", "config", r#"
        <h1>Configuration</h1>
        <form id="config-form" onsubmit="saveConfig(event)">
            <label>Base URL<input id="cfg-base-url" type="text"></label>
            <label>Bind Address<input id="cfg-bind" type="text"></label>
            <label>Toolchain<input id="cfg-toolchain" type="text"></label>
            <fieldset>
                <legend>Target Platforms</legend>
                <label><input type="checkbox" value="x86_64-unknown-linux-gnu" class="target-cb"> x86_64-unknown-linux-gnu</label>
                <label><input type="checkbox" value="aarch64-unknown-linux-gnu" class="target-cb"> aarch64-unknown-linux-gnu</label>
                <label><input type="checkbox" value="x86_64-pc-windows-msvc" class="target-cb"> x86_64-pc-windows-msvc</label>
                <label><input type="checkbox" value="x86_64-apple-darwin" class="target-cb"> x86_64-apple-darwin</label>
                <label><input type="checkbox" value="aarch64-apple-darwin" class="target-cb"> aarch64-apple-darwin</label>
            </fieldset>
            <fieldset>
                <legend>Behavior</legend>
                <label><input type="checkbox" id="cfg-watch"> Watch incoming</label>
                <label><input type="checkbox" id="cfg-verify"> Verify checksums</label>
                <label><input type="checkbox" id="cfg-keep-failed"> Keep failed packages</label>
                <label><input type="checkbox" id="cfg-prune"> Prune on import</label>
            </fieldset>
            <button class="primary" type="submit">Save Configuration</button>
        </form>
        <script>
        fetch('/api/config').then(r=>r.json()).then(c=>{
            document.getElementById('cfg-base-url').value=c.base_url||'';
            document.getElementById('cfg-bind').value=c.bind||'';
            document.getElementById('cfg-toolchain').value=c.toolchain||'';
            document.getElementById('cfg-watch').checked=c.watch_incoming;
            document.getElementById('cfg-verify').checked=c.verify_checksums;
            document.getElementById('cfg-keep-failed').checked=c.keep_failed_packages;
            document.getElementById('cfg-prune').checked=c.prune_on_import;
            (c.targets||[]).forEach(t=>{
                const cb=document.querySelector(`.target-cb[value="${t}"]`);
                if(cb)cb.checked=true;
            });
        });
        function saveConfig(e){
            e.preventDefault();
            const targets=[...document.querySelectorAll('.target-cb:checked')].map(c=>c.value);
            const cfg={
                base_url:document.getElementById('cfg-base-url').value,
                bind:document.getElementById('cfg-bind').value,
                toolchain:document.getElementById('cfg-toolchain').value,
                targets,
                watch_incoming:document.getElementById('cfg-watch').checked,
                verify_checksums:document.getElementById('cfg-verify').checked,
                keep_failed_packages:document.getElementById('cfg-keep-failed').checked,
                prune_on_import:document.getElementById('cfg-prune').checked,
            };
            fetch('/api/config',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(cfg)})
            .then(r=>r.json()).then(d=>{alert(d.ok?'Saved!':'Error: '+d.error)});
        }
        </script>
    "#))
}

async fn packages_page() -> Html<String> {
    Html(page_wrapper("Packages", "packages", r#"
        <h1>Packages</h1>
        <button onclick="runGc()">Run Garbage Collection</button>
        <table id="pkg-table">
            <thead><tr><th>Filename</th><th>Size</th><th>Status</th></tr></thead>
            <tbody></tbody>
        </table>
        <script>
        fetch('/api/packages').then(r=>r.json()).then(pkgs=>{
            const tb=document.querySelector('#pkg-table tbody');
            pkgs.forEach(p=>{
                const tr=document.createElement('tr');
                tr.className=p.status==='failed'?'row-failed':'';
                tr.innerHTML=`<td>${p.filename}</td><td>${(p.size/1024).toFixed(1)} KB</td><td>${p.status}</td>`;
                tb.appendChild(tr);
            });
        });
        function runGc(){
            fetch('/api/gc',{method:'POST'}).then(r=>r.json()).then(d=>{
                if(d.error)alert('Error: '+d.error);
                else alert(`GC complete: removed ${d.removed} crates, freed ${(d.freed_bytes/1024/1024).toFixed(1)} MB`);
            });
        }
        </script>
    "#))
}

async fn setup_page() -> Html<String> {
    Html(page_wrapper("Client Setup", "setup", r#"
        <h1>Client Setup</h1>
        <p>Configure client machines to use this frostmirror registry.</p>

        <h2>1. Environment Variables</h2>
        <pre id="env-snippet"></pre>

        <h2>2. Install Rustup</h2>
        <pre id="rustup-snippet"></pre>

        <h2>3. Configure Cargo</h2>
        <pre id="cargo-snippet"></pre>

        <h2>Downloads</h2>
        <a class="btn" href="/api/setup/cargo-config">cargo config.toml</a>
        <a class="btn" href="/api/setup/rustup-env.sh">rustup-env.sh</a>
        <a class="btn" href="/api/setup/rustup-env.ps1">rustup-env.ps1</a>

        <script>
        fetch('/api/config').then(r=>r.json()).then(c=>{
            const base=c.base_url||location.origin;
            document.getElementById('env-snippet').textContent=
                `export RUSTUP_DIST_SERVER=${base}\nexport RUSTUP_UPDATE_ROOT=${base}/rustup`;
            document.getElementById('rustup-snippet').textContent=
                `curl ${base}/rustup/dist/x86_64-unknown-linux-gnu/rustup-init -o rustup-init\nchmod +x rustup-init && ./rustup-init`;
            document.getElementById('cargo-snippet').textContent=
                `[http]\ncheck-revoke = false # may be needed if you have a self sign ssl https server\n\n[source.frostmirror]\nregistry = "sparse+${base}/index/"\n\n[source.crates-io]\nreplace-with = "frostmirror"`;
        });
        </script>
    "#))
}

async fn stylesheet() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css")],
        include_str!("../assets/style.css"),
    )
}

async fn script() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("../assets/app.js"),
    )
}
