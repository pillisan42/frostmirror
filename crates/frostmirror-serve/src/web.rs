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
            <label>Toolchain<input id="cfg-toolchain" type="text" list="toolchain-options" placeholder="stable | beta | nightly | 1.86.0"></label>
            <datalist id="toolchain-options">
                <option value="stable">
                <option value="beta">
                <option value="nightly">
                <option value="1.86.0">
                <option value="1.85.0">
                <option value="1.84.0">
            </datalist>
            <fieldset>
                <legend>Target Platforms</legend>
                <p class="hint">Tier 1 &amp; Tier 2 targets with host tools (full rustup toolchain available). Cross-compile-only targets must be added via <code>rustup target add</code> on the offline machine.</p>
                <details open>
                    <summary>Tier 1 (host tools)</summary>
                    <label><input type="checkbox" value="x86_64-unknown-linux-gnu" class="target-cb"> x86_64-unknown-linux-gnu</label>
                    <label><input type="checkbox" value="aarch64-unknown-linux-gnu" class="target-cb"> aarch64-unknown-linux-gnu</label>
                    <label><input type="checkbox" value="i686-unknown-linux-gnu" class="target-cb"> i686-unknown-linux-gnu</label>
                    <label><input type="checkbox" value="x86_64-pc-windows-msvc" class="target-cb"> x86_64-pc-windows-msvc</label>
                    <label><input type="checkbox" value="x86_64-pc-windows-gnu" class="target-cb"> x86_64-pc-windows-gnu</label>
                    <label><input type="checkbox" value="i686-pc-windows-msvc" class="target-cb"> i686-pc-windows-msvc</label>
                    <label><input type="checkbox" value="aarch64-pc-windows-msvc" class="target-cb"> aarch64-pc-windows-msvc</label>
                    <label><input type="checkbox" value="x86_64-apple-darwin" class="target-cb"> x86_64-apple-darwin</label>
                    <label><input type="checkbox" value="aarch64-apple-darwin" class="target-cb"> aarch64-apple-darwin</label>
                </details>
                <details>
                    <summary>Tier 2 — Linux (host tools)</summary>
                    <label><input type="checkbox" value="x86_64-unknown-linux-musl" class="target-cb"> x86_64-unknown-linux-musl</label>
                    <label><input type="checkbox" value="aarch64-unknown-linux-musl" class="target-cb"> aarch64-unknown-linux-musl</label>
                    <label><input type="checkbox" value="aarch64-unknown-linux-ohos" class="target-cb"> aarch64-unknown-linux-ohos</label>
                    <label><input type="checkbox" value="x86_64-unknown-linux-ohos" class="target-cb"> x86_64-unknown-linux-ohos</label>
                    <label><input type="checkbox" value="arm-unknown-linux-gnueabi" class="target-cb"> arm-unknown-linux-gnueabi</label>
                    <label><input type="checkbox" value="arm-unknown-linux-gnueabihf" class="target-cb"> arm-unknown-linux-gnueabihf</label>
                    <label><input type="checkbox" value="armv7-unknown-linux-gnueabihf" class="target-cb"> armv7-unknown-linux-gnueabihf</label>
                    <label><input type="checkbox" value="armv7-unknown-linux-ohos" class="target-cb"> armv7-unknown-linux-ohos</label>
                    <label><input type="checkbox" value="loongarch64-unknown-linux-gnu" class="target-cb"> loongarch64-unknown-linux-gnu</label>
                    <label><input type="checkbox" value="loongarch64-unknown-linux-musl" class="target-cb"> loongarch64-unknown-linux-musl</label>
                    <label><input type="checkbox" value="powerpc-unknown-linux-gnu" class="target-cb"> powerpc-unknown-linux-gnu</label>
                    <label><input type="checkbox" value="powerpc64-unknown-linux-gnu" class="target-cb"> powerpc64-unknown-linux-gnu</label>
                    <label><input type="checkbox" value="powerpc64-unknown-linux-musl" class="target-cb"> powerpc64-unknown-linux-musl</label>
                    <label><input type="checkbox" value="powerpc64le-unknown-linux-gnu" class="target-cb"> powerpc64le-unknown-linux-gnu</label>
                    <label><input type="checkbox" value="powerpc64le-unknown-linux-musl" class="target-cb"> powerpc64le-unknown-linux-musl</label>
                    <label><input type="checkbox" value="riscv64gc-unknown-linux-gnu" class="target-cb"> riscv64gc-unknown-linux-gnu</label>
                    <label><input type="checkbox" value="s390x-unknown-linux-gnu" class="target-cb"> s390x-unknown-linux-gnu</label>
                </details>
                <details>
                    <summary>Tier 2 — Windows / BSD / Solaris (host tools)</summary>
                    <label><input type="checkbox" value="i686-pc-windows-gnu" class="target-cb"> i686-pc-windows-gnu</label>
                    <label><input type="checkbox" value="aarch64-pc-windows-gnullvm" class="target-cb"> aarch64-pc-windows-gnullvm</label>
                    <label><input type="checkbox" value="x86_64-pc-windows-gnullvm" class="target-cb"> x86_64-pc-windows-gnullvm</label>
                    <label><input type="checkbox" value="x86_64-unknown-freebsd" class="target-cb"> x86_64-unknown-freebsd</label>
                    <label><input type="checkbox" value="x86_64-unknown-netbsd" class="target-cb"> x86_64-unknown-netbsd</label>
                    <label><input type="checkbox" value="x86_64-unknown-illumos" class="target-cb"> x86_64-unknown-illumos</label>
                    <label><input type="checkbox" value="x86_64-pc-solaris" class="target-cb"> x86_64-pc-solaris</label>
                    <label><input type="checkbox" value="sparcv9-sun-solaris" class="target-cb"> sparcv9-sun-solaris</label>
                </details>
                <p class="hint"><a href="https://doc.rust-lang.org/nightly/rustc/platform-support.html" target="_blank" rel="noopener">Full target list (Tier 1, 2, 3) at rust-lang docs &rarr;</a></p>
            </fieldset>
            <fieldset>
                <legend>Behavior</legend>
                <label><input type="checkbox" id="cfg-watch"> Watch incoming</label>
                <label><input type="checkbox" id="cfg-verify"> Verify checksums</label>
                <label><input type="checkbox" id="cfg-keep-failed"> Keep failed packages</label>
                <label><input type="checkbox" id="cfg-prune"> Prune on import</label>
            </fieldset>
            <fieldset>
                <legend>Live-mirror (proxy upstream)</legend>
                <p class="hint">When enabled, missing crates and toolchain files are fetched from upstream on demand and cached locally. Leave off for strict offline / air-gapped operation. Restart the server after changing this setting.</p>
                <label><input type="checkbox" id="cfg-proxy-mode"> Enable live-mirror</label>
                <details id="cfg-proxy-details">
                    <summary>Upstream URLs</summary>
                    <label>Sparse index URL<input id="cfg-proxy-index" type="text" placeholder="https://index.crates.io"></label>
                    <label>Crate download URL<input id="cfg-proxy-dl" type="text" placeholder="https://static.crates.io/crates"></label>
                    <label>Toolchain dist URL<input id="cfg-proxy-dist" type="text" placeholder="https://static.rust-lang.org"></label>
                </details>
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
            document.getElementById('cfg-proxy-mode').checked=!!c.proxy_mode;
            document.getElementById('cfg-proxy-index').value=c.proxy_index_url||'';
            document.getElementById('cfg-proxy-dl').value=c.proxy_dl_url||'';
            document.getElementById('cfg-proxy-dist').value=c.proxy_dist_url||'';
            document.getElementById('cfg-proxy-details').open=!!c.proxy_mode;
            (c.targets||[]).forEach(t=>{
                const cb=document.querySelector(`.target-cb[value="${t}"]`);
                if(cb)cb.checked=true;
            });
        });
        document.getElementById('cfg-proxy-mode').addEventListener('change',e=>{
            document.getElementById('cfg-proxy-details').open=e.target.checked;
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
                proxy_mode:document.getElementById('cfg-proxy-mode').checked,
                proxy_index_url:document.getElementById('cfg-proxy-index').value||'https://index.crates.io',
                proxy_dl_url:document.getElementById('cfg-proxy-dl').value||'https://static.crates.io/crates',
                proxy_dist_url:document.getElementById('cfg-proxy-dist').value||'https://static.rust-lang.org',
            };
            fetch('/api/config',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(cfg)})
            .then(r=>r.json()).then(d=>{alert(d.ok?'Saved! Restart the server for live-mirror changes to take effect.':'Error: '+d.error)});
        }
        </script>
    "#))
}

async fn packages_page() -> Html<String> {
    Html(page_wrapper("Packages", "packages", r#"
        <h1>Packages</h1>
        <button onclick="runGc()">Run Garbage Collection</button>
        <button id="snapshot-btn" onclick="createSnapshot()">Create Snapshot</button>

        <h2>Imported / Failed</h2>
        <table id="pkg-table">
            <thead><tr><th>Filename</th><th>Size</th><th>Status</th></tr></thead>
            <tbody></tbody>
        </table>

        <h2>Snapshots</h2>
        <p class="hint">Self-contained <code>.pkg</code> bundles of this server's mirror + configuration. Drop one into the <code>incoming/</code> directory of another frostmirror server to redeploy.</p>
        <table id="snap-table">
            <thead><tr><th>Filename</th><th>Size</th><th>Created</th><th></th></tr></thead>
            <tbody></tbody>
        </table>

        <script>
        function fmtSize(n){
            if(n<1024)return n+' B';
            if(n<1024*1024)return (n/1024).toFixed(1)+' KB';
            if(n<1024*1024*1024)return (n/1024/1024).toFixed(1)+' MB';
            return (n/1024/1024/1024).toFixed(2)+' GB';
        }
        fetch('/api/packages').then(r=>r.json()).then(pkgs=>{
            const tb=document.querySelector('#pkg-table tbody');
            pkgs.forEach(p=>{
                const tr=document.createElement('tr');
                tr.className=p.status==='failed'?'row-failed':'';
                tr.innerHTML=`<td>${p.filename}</td><td>${fmtSize(p.size)}</td><td>${p.status}</td>`;
                tb.appendChild(tr);
            });
        });
        function loadSnapshots(){
            fetch('/api/export').then(r=>r.json()).then(snaps=>{
                const tb=document.querySelector('#snap-table tbody');
                tb.innerHTML='';
                snaps.forEach(s=>{
                    const tr=document.createElement('tr');
                    const created=s.created?new Date(s.created).toLocaleString():'-';
                    tr.innerHTML=`<td>${s.filename}</td><td>${fmtSize(s.size)}</td><td>${created}</td><td><a href="/api/export/download/${encodeURIComponent(s.filename)}">Download</a></td>`;
                    tb.appendChild(tr);
                });
            });
        }
        loadSnapshots();
        function runGc(){
            fetch('/api/gc',{method:'POST'}).then(r=>r.json()).then(d=>{
                if(d.error)alert('Error: '+d.error);
                else alert(`GC complete: removed ${d.removed} crates, freed ${fmtSize(d.freed_bytes)}`);
            });
        }
        function createSnapshot(){
            const btn=document.getElementById('snapshot-btn');
            btn.disabled=true;
            btn.textContent='Building snapshot...';
            fetch('/api/export',{method:'POST'}).then(r=>r.json()).then(d=>{
                btn.disabled=false;
                btn.textContent='Create Snapshot';
                if(d.error){alert('Error: '+d.error);return;}
                loadSnapshots();
                alert(`Snapshot created: ${d.filename} (${fmtSize(d.size)}, ${d.crate_count} crates)`);
            }).catch(e=>{
                btn.disabled=false;
                btn.textContent='Create Snapshot';
                alert('Snapshot failed: '+e);
            });
        }
        </script>
    "#))
}

async fn setup_page() -> Html<String> {
    Html(page_wrapper("Client Setup", "setup", r#"
        <h1>Client Setup</h1>
        <p>Configure client machines to use this frostmirror registry.</p>

        <div class="os-tabs" role="tablist">
            <button class="os-tab" data-os="linux" type="button">Linux</button>
            <button class="os-tab" data-os="macos" type="button">macOS</button>
            <button class="os-tab" data-os="windows" type="button">Windows</button>
        </div>

        <h2>1. Environment Variables</h2>
        <pre id="env-snippet"></pre>

        <h2>2. Install Rustup</h2>
        <pre id="rustup-snippet"></pre>
        <div class="os-note" id="msvc-note" hidden>
            <strong>Windows requires the MSVC toolchain.</strong>
            The <code>x86_64-pc-windows-msvc</code> target needs the Microsoft C++ build tools
            (linker + Windows SDK) installed separately — frostmirror does not ship them.
            Install <a href="https://visualstudio.microsoft.com/visual-cpp-build-tools/" target="_blank" rel="noopener">Build Tools for Visual Studio</a>
            (select the <em>Desktop development with C++</em> workload) before running <code>cargo build</code>.
        </div>

        <h2>3. Configure Cargo</h2>
        <pre id="cargo-snippet"></pre>

        <h2>4. SSL Revocation Check (optional)</h2>
        <p>If cargo fails with TLS or certificate-revocation errors against this
        mirror &mdash; common on Windows when CRL/OCSP endpoints are blocked,
        or when the mirror uses a self-signed certificate &mdash; append the
        following to your <code>.cargo/config.toml</code>:</p>
        <pre>[http]
check-revoke = false</pre>
        <p class="hint">Cargo's default for <code>check-revoke</code> is
        <code>true</code> on Windows and <code>false</code> elsewhere. Only set
        it to <code>false</code> when revocation checking is the actual blocker;
        it disables a security feature.</p>

        <h2>Downloads</h2>
        <a class="btn" href="/api/setup/cargo-config">cargo config.toml</a>
        <a class="btn" href="/api/setup/cargo-config?check_revoke=false">cargo config.toml (skip revocation)</a>
        <a class="btn" href="/api/setup/rustup-env.sh">rustup-env.sh</a>
        <a class="btn" href="/api/setup/rustup-env.ps1">rustup-env.ps1</a>

        <script>
        function detectOs(){
            const p=(navigator.userAgentData&&navigator.userAgentData.platform)||navigator.platform||navigator.userAgent||'';
            const s=p.toLowerCase();
            if(s.includes('win'))return 'windows';
            if(s.includes('mac')||s.includes('darwin'))return 'macos';
            return 'linux';
        }
        function snippets(base,os){
            const cargo=`[source.frostmirror]\nregistry = "sparse+${base}/index/"\n\n[source.crates-io]\nreplace-with = "frostmirror"`;
            if(os==='windows'){
                return {
                    env:`$env:RUSTUP_DIST_SERVER = "${base}"\n$env:RUSTUP_UPDATE_ROOT = "${base}/rustup"`,
                    rustup:`Invoke-WebRequest "${base}/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe" -OutFile rustup-init.exe\n.\\rustup-init.exe`,
                    cargo,
                };
            }
            const triple=os==='macos'?'x86_64-apple-darwin':'x86_64-unknown-linux-gnu';
            return {
                env:`export RUSTUP_DIST_SERVER=${base}\nexport RUSTUP_UPDATE_ROOT=${base}/rustup`,
                rustup:`curl ${base}/rustup/dist/${triple}/rustup-init -o rustup-init\nchmod +x rustup-init && ./rustup-init`,
                cargo,
            };
        }
        fetch('/api/config').then(r=>r.json()).then(c=>{
            const base=c.base_url||location.origin;
            const tabs=document.querySelectorAll('.os-tab');
            function render(os){
                const s=snippets(base,os);
                document.getElementById('env-snippet').textContent=s.env;
                document.getElementById('rustup-snippet').textContent=s.rustup;
                document.getElementById('cargo-snippet').textContent=s.cargo;
                document.getElementById('msvc-note').hidden=os!=='windows';
                tabs.forEach(t=>t.classList.toggle('active',t.dataset.os===os));
            }
            tabs.forEach(t=>t.addEventListener('click',()=>render(t.dataset.os)));
            render(detectOs());
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
