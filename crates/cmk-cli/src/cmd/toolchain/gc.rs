use anyhow::Result;
use cmk_core::store::Store;
use cmk_toolchain::shim;

pub fn run(keep: Option<usize>) -> Result<()> {
    let store = Store::open()?;
    let mut bytes_freed: u64 = 0;
    for d in [store.downloads(), store.build_cache()] {
        if d.is_dir() {
            bytes_freed += dir_size(&d).unwrap_or(0);
            std::fs::remove_dir_all(&d)?;
            std::fs::create_dir_all(&d)?;
        }
    }
    println!(
        "freed ~{} MiB from downloads/ + build-cache/",
        bytes_freed / (1024 * 1024)
    );

    if let Some(n) = keep {
        let removed = retain_versions(&store, n)?;
        for r in &removed {
            println!("evicted {r}");
        }
        if !removed.is_empty() {
            let self_exe = std::env::current_exe()?;
            let shim_bin = shim::locate_shim_binary(&self_exe);
            shim::rebuild_shims(&store, &shim_bin)?;
        }
    }
    Ok(())
}

fn retain_versions(store: &Store, keep: usize) -> Result<Vec<String>> {
    let mut idx = store.read_installed()?;
    let mut entries: Vec<(String, String)> = idx
        .versions
        .iter()
        .map(|(key, v)| {
            let last_installed = v
                .packages
                .values()
                .map(|p| p.installed_at.clone())
                .max()
                .unwrap_or_default();
            (key.clone(), last_installed)
        })
        .collect();
    // Newest installed_at last; keep the tail of length `keep`.
    entries.sort_by(|a, b| a.1.cmp(&b.1));
    if entries.len() <= keep {
        return Ok(vec![]);
    }
    let to_drop = entries.len() - keep;
    let evicted: Vec<String> = entries.iter().take(to_drop).map(|(k, _)| k.clone()).collect();

    let current = store.read_current()?.unwrap_or_default();
    for key in &evicted {
        if let Some(inst) = idx.versions.remove(key) {
            let dir = store.version_dir(&inst.version, &inst.platform);
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            if inst.version == current {
                let _ = std::fs::remove_file(store.current_path());
            }
        }
    }
    store.write_installed(&idx)?;
    Ok(evicted)
}

fn dir_size(p: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                if let Ok(ft) = e.file_type() {
                    if ft.is_dir() {
                        stack.push(e.path());
                    } else if ft.is_file()
                        && let Ok(meta) = e.metadata()
                    {
                        total += meta.len();
                    }
                }
            }
        }
    }
    Ok(total)
}
