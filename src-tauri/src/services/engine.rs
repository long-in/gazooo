//! エンジン接続の管理・監視(docs/04-ipc-api.md「接続状態の監視」)。

use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::engine::comfyui::ComfyUiClient;
use crate::engine::{EngineFailure, EngineModels, EngineStatus, GenerationEngine};
use crate::error::{AppError, AppResult};

pub const STATUS_EVENT: &str = "engine://status";
const POLL_INTERVAL: Duration = Duration::from_secs(5);

pub struct EngineService {
    engine: RwLock<Arc<dyn GenerationEngine>>,
    last_emitted: Mutex<Option<EngineStatus>>,
}

impl EngineService {
    pub fn new(url: &str) -> AppResult<Self> {
        Ok(Self {
            engine: RwLock::new(Arc::new(ComfyUiClient::new(url)?)),
            last_emitted: Mutex::new(None),
        })
    }

    /// 既存のエンジン実装から作る(テスト用)。
    #[cfg(test)]
    pub fn from_engine(engine: Arc<dyn GenerationEngine>) -> Self {
        Self {
            engine: RwLock::new(engine),
            last_emitted: Mutex::new(None),
        }
    }

    /// 現在のエンジン。
    pub fn engine(&self) -> AppResult<Arc<dyn GenerationEngine>> {
        self.engine
            .read()
            .map(|e| Arc::clone(&e))
            .map_err(|_| AppError::internal("engine lock poisoned"))
    }

    /// 直近に確認できた機能の有無(未確認・未接続なら `None`)。通信はしない。
    pub fn last_features(&self) -> Option<crate::engine::EngineFeatures> {
        let g = self.last_emitted.lock().ok()?;
        g.as_ref()
            .filter(|s| s.connected)
            .map(|s| s.features.clone())
    }

    /// 接続先 URL を差し替える。変更があれば `true`。
    pub fn set_url(&self, url: &str) -> AppResult<bool> {
        let url = url.trim_end_matches('/');
        if self.engine()?.url() == url {
            return Ok(false);
        }
        let new = Arc::new(ComfyUiClient::new(url)?);
        *self
            .engine
            .write()
            .map_err(|_| AppError::internal("engine lock poisoned"))? = new;
        Ok(true)
    }

    /// ヘルスチェック。接続できなくてもエラーにせず、切断状態として返す。
    pub async fn check(&self) -> AppResult<EngineStatus> {
        let engine = self.engine()?;
        let started = Instant::now();
        Ok(match engine.health().await {
            Ok(status) => {
                tracing::debug!(
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "engine health check ok"
                );
                status
            }
            Err(e) => {
                tracing::debug!(
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    code = e.code(),
                    error = %e,
                    "engine health check failed"
                );
                match e {
                    AppError::EngineBadResponse(_) => {
                        EngineStatus::disconnected(engine.url(), EngineFailure::BadResponse)
                    }
                    _ => EngineStatus::disconnected(engine.url(), EngineFailure::Unreachable),
                }
            }
        })
    }

    pub async fn list_models(&self) -> AppResult<EngineModels> {
        self.engine()?.list_models().await
    }

    /// 状態を確認し、前回通知から変化があった場合(または `force`)に通知する。
    pub async fn refresh(&self, app: &AppHandle, force: bool) -> AppResult<EngineStatus> {
        let status = self.check().await?;
        let changed = {
            let mut last = self
                .last_emitted
                .lock()
                .map_err(|_| AppError::internal("engine state lock poisoned"))?;
            let changed = !last.as_ref().is_some_and(|prev| same_state(prev, &status));
            if changed {
                *last = Some(status.clone());
            }
            changed
        };
        if changed || force {
            let _ = app.emit(STATUS_EVENT, &status);
            tracing::info!(connected = status.connected, url = %status.url, "engine status");
        }
        Ok(status)
    }
}

/// 通知が必要な変化がないか。空きメモリ(`vram_free`)は生成のたびに揺れるため、比較から除く。
fn same_state(a: &EngineStatus, b: &EngineStatus) -> bool {
    let strip = |s: &EngineStatus| EngineStatus {
        vram_free: None,
        ..s.clone()
    };
    strip(a) == strip(b)
}

/// 自動復旧の通知(docs/47 §4.4)。
pub const RECOVERY_EVENT: &str = "engine://recovery";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryEvent {
    phase: &'static str,
    attempt: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    requeued_jobs: Option<usize>,
}

/// 一定間隔でエンジンの状態を監視するバックグラウンドタスク。
/// 自動復旧が ON のときは、切断が続いたら ComfyUI を再起動する(docs/47)。
pub async fn monitor(app: AppHandle) {
    use crate::db::Db;
    use crate::services::engine_process::{EngineProcess, ProcStatus};
    use crate::services::engine_recovery::{Action, RecoveryTracker};
    use crate::services::jobs::JobService;

    let mut failed_checks = 0u32;
    let mut tracker = RecoveryTracker::new();
    loop {
        if let Some(svc) = app.try_state::<Arc<EngineService>>()
            && let Ok(status) = svc.refresh(&app, false).await
        {
            let proc = app.try_state::<Arc<EngineProcess>>();
            if status.connected {
                if failed_checks > 0 {
                    tracing::debug!(failed_checks, "engine reachable again");
                }
                failed_checks = 0;
            } else {
                failed_checks += 1;
                // 切断と判定したとき、アプリが起動した ComfyUI のプロセスが生きているかを残す(原因の切り分け用)
                tracing::debug!(
                    consecutive_failures = failed_checks,
                    process = ?proc.as_ref().map(|p| p.state()),
                    "engine considered disconnected"
                );
            }

            // 自動復旧(docs/47)。設定は毎回読む(切り替えがすぐ効くように)
            let cfg = match app.try_state::<Db>() {
                Some(db) => db
                    .call_device(|c| crate::services::settings::get_managed_engine(c))
                    .await
                    .ok(),
                None => None,
            };
            let enabled = cfg.as_ref().is_some_and(|c| c.enabled && c.auto_recover);
            let eligible = enabled
                && proc.as_ref().is_some_and(|p| {
                    matches!(p.state().status, ProcStatus::Running | ProcStatus::Crashed)
                });
            let action = tracker.observe(Instant::now(), status.connected, eligible);
            if let Some(jobs) = app.try_state::<Arc<JobService>>() {
                jobs.set_recovery(tracker.hold(), enabled);
            }
            if let (Action::Restart { attempt }, Some(cfg), Some(proc)) = (action, cfg, proc) {
                recover(&app, &mut tracker, &proc, &cfg, attempt).await;
            }
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// ComfyUI を再起動し、結果を通知する。
async fn recover(
    app: &AppHandle,
    tracker: &mut crate::services::engine_recovery::RecoveryTracker,
    proc: &Arc<crate::services::engine_process::EngineProcess>,
    cfg: &crate::services::engine_process::config::ManagedEngineConfig,
    attempt: u32,
) {
    use crate::services::engine_process::ProcStatus;
    use crate::services::jobs::JobService;
    tracing::warn!(
        attempt,
        "engine disconnected for 3 minutes; restarting ComfyUI"
    );
    let _ = app.emit(
        RECOVERY_EVENT,
        &RecoveryEvent {
            phase: "restarting",
            attempt,
            requeued_jobs: None,
        },
    );
    let result = proc.restart(cfg).await;
    let ok = matches!(&result, Ok(s) if s.status == ProcStatus::Running);
    let fatal = matches!(
        &result,
        Err(e) if matches!(e.code(), "ENGINE_PORT_IN_USE" | "ENGINE_LAUNCH_FAILED")
    );
    if let Err(e) = &result {
        tracing::warn!(code = e.code(), error = %e, "automatic restart failed");
    }
    let gave_up = tracker.restart_finished(Instant::now(), ok, fatal);
    if let Some(jobs) = app.try_state::<Arc<JobService>>() {
        jobs.set_recovery(tracker.hold(), true);
    }
    let (phase, requeued_jobs) = if ok {
        let n = match app.try_state::<Arc<JobService>>() {
            Some(j) => j.active_counts().await.map(|(r, q)| r + q).unwrap_or(0),
            None => 0,
        };
        ("recovered", Some(n))
    } else if gave_up {
        ("gave_up", None)
    } else {
        // まだ試行が残っている。次の 3 分のあとにまた試す
        return;
    };
    let _ = app.emit(
        RECOVERY_EVENT,
        &RecoveryEvent {
            phase,
            attempt,
            requeued_jobs,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::params::Preprocessor;
    use crate::engine::EngineFeatures;

    fn connected() -> EngineStatus {
        EngineStatus {
            connected: true,
            url: "http://127.0.0.1:8188".into(),
            version: Some("0.45.6".into()),
            device: Some("mps".into()),
            vram_free: Some(100),
            features: EngineFeatures {
                gguf_node: true,
                face_detailer: true,
                controlnet_preprocessors: vec![Preprocessor::Canny],
                inpaint: true,
                img2img: true,
                reference_edit: true,
                face_swap: true,
                face_swap_composite: true,
                image_tag: true,
            },
            error: None,
        }
    }

    #[test]
    fn vram_fluctuation_is_not_a_change() {
        let a = connected();
        let b = EngineStatus {
            vram_free: Some(999),
            ..connected()
        };
        assert!(same_state(&a, &b));
    }

    #[test]
    fn meaningful_differences_are_changes() {
        let a = connected();
        let disconnected = EngineStatus::disconnected(a.url.clone(), EngineFailure::Unreachable);
        assert!(!same_state(&a, &disconnected));
        let no_gguf = EngineStatus {
            features: EngineFeatures {
                gguf_node: false,
                ..connected().features
            },
            ..connected()
        };
        assert!(!same_state(&a, &no_gguf));
        let no_face = EngineStatus {
            features: EngineFeatures {
                face_detailer: false,
                ..connected().features
            },
            ..connected()
        };
        assert!(!same_state(&a, &no_face));
        let no_cn = EngineStatus {
            features: EngineFeatures {
                controlnet_preprocessors: vec![],
                ..connected().features
            },
            ..connected()
        };
        assert!(!same_state(&a, &no_cn));
    }
}
