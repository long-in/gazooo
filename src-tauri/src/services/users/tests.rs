use super::*;
use crate::services::settings;

struct H {
    dir: std::path::PathBuf,
    db: Db,
    paths: AppPaths,
    svc: Arc<UserService>,
}

impl Drop for H {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn harness_in(dir: std::path::PathBuf) -> H {
    let app_db = dir.join("app.db");
    let db = Db::new(&app_db).unwrap();
    let paths = AppPaths::new(dir.clone());
    let svc = UserService::new(&app_db, db.clone(), paths.clone()).unwrap();
    H {
        dir,
        db,
        paths,
        svc,
    }
}

fn temp_dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("gazooo-users-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn harness() -> H {
    harness_in(temp_dir())
}

async fn add(h: &H, name: &str) -> UserSummary {
    h.svc.create(name.into(), "blue".into()).await.unwrap()
}

fn code(e: AppError) -> &'static str {
    e.code()
}

async fn preset_count(h: &H) -> i64 {
    h.db.call(|c| Ok(c.query_row("SELECT COUNT(*) FROM presets", [], |r| r.get(0))?))
        .await
        .unwrap()
}

#[tokio::test]
async fn data_calls_require_login_and_workspaces_are_isolated() {
    let h = harness();
    assert_eq!(
        code(h.db.call(|c| settings::load(c)).await.unwrap_err()),
        "NOT_LOGGED_IN"
    );
    let a = add(&h, "A").await;
    let b = add(&h, "B").await;

    h.svc.login(a.id.clone(), None).await.unwrap();
    h.db.call(|c| {
        c.execute(
            "INSERT INTO presets (id,name,params_json,created_at,updated_at) VALUES ('p','mine','{}',0,0)",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let dir_a = h.paths.default_images_dir();
    assert!(dir_a.starts_with(h.paths.user_workspace(&a.id)));
    h.svc.logout().unwrap();
    assert_eq!(
        code(h.db.call(|c| settings::load(c)).await.unwrap_err()),
        "NOT_LOGGED_IN"
    );

    h.svc.login(b.id.clone(), None).await.unwrap();
    assert_eq!(preset_count(&h).await, 0, "B must not see A's presets");
    assert_ne!(h.paths.default_images_dir(), dir_a);
    h.svc.logout().unwrap();

    h.svc.login(a.id, None).await.unwrap();
    assert_eq!(preset_count(&h).await, 1);
}

#[tokio::test]
async fn device_settings_are_shared_and_user_settings_are_separate() {
    let h = harness();
    let a = add(&h, "A").await;
    let b = add(&h, "B").await;
    // 未ログインでも端末共通の設定は読み書きできる
    h.db.call_device(|c| settings::set_debug_enabled(c, true))
        .await
        .unwrap();

    h.svc.login(a.id.clone(), None).await.unwrap();
    assert!(h.db.call(|c| settings::get_debug_enabled(c)).await.unwrap());
    h.db.call(|c| {
        c.execute(
            "INSERT INTO settings (key,value_json,updated_at) VALUES ('ui.theme','\"light\"',0)",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    // 端末共通のキーはユーザーの settings には入らない
    let n: i64 =
        h.db.call(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM main.settings WHERE key = 'debug.enabled'",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(n, 0);
    h.svc.logout().unwrap();

    h.svc.login(b.id, None).await.unwrap();
    assert!(h.db.call(|c| settings::get_debug_enabled(c)).await.unwrap());
    let theme = h.db.call(|c| settings::load(c)).await.unwrap().theme;
    assert_eq!(theme, settings::Theme::Dark, "B keeps the default theme");
}

#[tokio::test]
async fn names_are_unique_case_insensitive_and_limited() {
    let h = harness();
    add(&h, "Taro").await;
    assert_eq!(
        code(h.svc.create("taro".into(), "red".into()).await.unwrap_err()),
        "USER_NAME_TAKEN"
    );
    assert_eq!(
        code(h.svc.create("  ".into(), "red".into()).await.unwrap_err()),
        "VALIDATION"
    );
    assert_eq!(
        code(h.svc.create("x".into(), "nope".into()).await.unwrap_err()),
        "VALIDATION"
    );
    for i in 0..(MAX_USERS - 1) {
        add(&h, &format!("u{i}")).await;
    }
    assert_eq!(
        code(h.svc.create("over".into(), "red".into()).await.unwrap_err()),
        "USER_LIMIT"
    );
}

#[tokio::test]
async fn lock_is_off_by_default_and_requires_current_value_to_change() {
    let h = harness();
    let a = add(&h, "A").await;
    assert_eq!(a.auth_kind, AuthKind::None);
    h.svc.login(a.id.clone(), None).await.unwrap();

    // 形式の誤りは失敗回数を増やさない
    assert_eq!(
        code(
            h.svc
                .lock_set(AuthKind::Pin, "12".into(), Some(4), None)
                .await
                .unwrap_err()
        ),
        "VALIDATION"
    );
    let u = h
        .svc
        .lock_set(AuthKind::Pin, "1234".into(), Some(4), None)
        .await
        .unwrap();
    assert_eq!(u.auth_kind, AuthKind::Pin);
    assert_eq!(u.pin_length, Some(4));

    // 変更・解除には現在の値が必要
    assert_eq!(
        code(
            h.svc
                .lock_set(AuthKind::Passphrase, "long passphrase".into(), None, None)
                .await
                .unwrap_err()
        ),
        "AUTH_FAILED"
    );
    let u = h
        .svc
        .lock_set(
            AuthKind::Passphrase,
            "long passphrase".into(),
            None,
            Some("1234".into()),
        )
        .await
        .unwrap();
    assert_eq!(u.auth_kind, AuthKind::Passphrase);
    assert_eq!(
        code(h.svc.lock_clear(Some("bad".into())).await.unwrap_err()),
        "AUTH_FAILED"
    );
    let u = h
        .svc
        .lock_clear(Some("long passphrase".into()))
        .await
        .unwrap();
    assert_eq!(u.auth_kind, AuthKind::None);
}

#[tokio::test]
async fn login_checks_secret_and_locks_out_after_repeated_failures() {
    let dir = temp_dir();
    let h = harness_in(dir.clone());
    let a = add(&h, "A").await;
    h.svc.login(a.id.clone(), None).await.unwrap();
    h.svc
        .lock_set(AuthKind::Pin, "4321".into(), Some(4), None)
        .await
        .unwrap();
    h.svc.logout().unwrap();

    // 入力なしも失敗として数える
    assert_eq!(
        code(h.svc.login(a.id.clone(), None).await.unwrap_err()),
        "AUTH_FAILED"
    );
    for _ in 0..3 {
        assert_eq!(
            code(
                h.svc
                    .login(a.id.clone(), Some("0000".into()))
                    .await
                    .unwrap_err()
            ),
            "AUTH_FAILED"
        );
    }
    // 5 回目で待機に入る
    let e = h
        .svc
        .login(a.id.clone(), Some("0000".into()))
        .await
        .unwrap_err();
    assert_eq!(e.code(), "AUTH_LOCKED_OUT");
    assert!(
        matches!(&e, AppError::User { details: Some(d), .. } if d["retryAfterMs"].as_i64().unwrap() > 20_000)
    );
    // 待機中は正しい値でも入れない
    assert_eq!(
        code(
            h.svc
                .login(a.id.clone(), Some("4321".into()))
                .await
                .unwrap_err()
        ),
        "AUTH_LOCKED_OUT"
    );
    assert!(h.svc.list().await.unwrap()[0].locked_until.is_some());

    // 再起動しても待機が残る(app.db に保存)
    drop(h);
    let h2 = harness_in(dir.clone());
    assert!(h2.svc.list().await.unwrap()[0].locked_until.is_some());

    // 待機が終われば、正しい値で入れて回数がリセットされる
    h2.svc
        .conn()
        .unwrap()
        .execute("UPDATE users SET locked_until = 1", [])
        .unwrap();
    h2.svc
        .login(a.id.clone(), Some("4321".into()))
        .await
        .unwrap();
    let failed: u32 = h2
        .svc
        .conn()
        .unwrap()
        .query_row("SELECT failed_count FROM users", [], |r| r.get(0))
        .unwrap();
    assert_eq!(failed, 0);
}

#[tokio::test]
async fn summaries_never_expose_the_hash() {
    let h = harness();
    let a = add(&h, "A").await;
    h.svc.login(a.id, None).await.unwrap();
    let u = h
        .svc
        .lock_set(AuthKind::Passphrase, "secret phrase".into(), None, None)
        .await
        .unwrap();
    let json = serde_json::to_string(&u).unwrap();
    assert!(!json.contains("argon2") && !json.contains("hash") && !json.contains("secret"));
}

#[tokio::test]
async fn delete_removes_workspace_and_guards_self_and_last() {
    let h = harness();
    let a = add(&h, "A").await;
    let b = add(&h, "B").await;
    h.svc.login(a.id.clone(), None).await.unwrap();
    h.svc.logout().unwrap();
    h.svc.login(b.id.clone(), None).await.unwrap();

    assert_eq!(
        code(h.svc.delete(b.id.clone(), None, None).await.unwrap_err()),
        "VALIDATION",
        "cannot delete the logged-in user"
    );
    let ws = h.paths.user_workspace(&a.id);
    assert!(ws.exists());
    h.svc.delete(a.id.clone(), None, None).await.unwrap();
    assert!(!ws.exists());
    h.svc.logout().unwrap();
    assert_eq!(
        code(h.svc.delete(b.id, None, None).await.unwrap_err()),
        "USER_LAST"
    );
}

#[tokio::test]
async fn locked_user_delete_needs_secret_or_exact_name() {
    let h = harness();
    let a = add(&h, "Alice").await;
    let b = add(&h, "B").await;
    h.svc.login(a.id.clone(), None).await.unwrap();
    h.svc
        .lock_set(AuthKind::Pin, "1111".into(), Some(4), None)
        .await
        .unwrap();
    h.svc.logout().unwrap();
    h.svc.login(b.id, None).await.unwrap();

    assert_eq!(
        code(
            h.svc
                .delete(a.id.clone(), None, Some("alice".into()))
                .await
                .unwrap_err()
        ),
        "VALIDATION"
    );
    assert_eq!(
        code(
            h.svc
                .delete(a.id.clone(), Some("2222".into()), None)
                .await
                .unwrap_err()
        ),
        "AUTH_FAILED"
    );
    h.svc
        .delete(a.id, None, Some("Alice".into()))
        .await
        .unwrap();
    assert_eq!(h.svc.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn rename_needs_secret_for_other_locked_users_only() {
    let h = harness();
    let a = add(&h, "A").await;
    let b = add(&h, "B").await;
    h.svc.login(a.id.clone(), None).await.unwrap();
    h.svc
        .lock_set(AuthKind::Pin, "1111".into(), Some(4), None)
        .await
        .unwrap();
    // 自分自身はロックの入力なしで変更できる
    h.svc.rename(a.id.clone(), "A2".into(), None).await.unwrap();
    h.svc.logout().unwrap();
    h.svc.login(b.id, None).await.unwrap();
    assert_eq!(
        code(
            h.svc
                .rename(a.id.clone(), "A3".into(), None)
                .await
                .unwrap_err()
        ),
        "AUTH_FAILED"
    );
    h.svc
        .rename(a.id, "A3".into(), Some("1111".into()))
        .await
        .unwrap();
}

#[tokio::test]
async fn legacy_data_moves_to_the_first_user_without_moving_images() {
    let dir = temp_dir();
    // 従来の単一データ(gazooo.db)を用意する。旧版では端末共通のキーも同じ settings にあった
    let legacy = dir.join("gazooo.db");
    {
        let db = Db::open(&legacy).unwrap();
        db.call(|c| {
            c.execute(
                "INSERT INTO presets (id,name,params_json,created_at,updated_at) VALUES ('p','old','{}',0,0)",
                [],
            )?;
            for (k, v) in [
                ("engine.comfyui.url", "\"http://10.0.0.5:8188\""),
                ("debug.enabled", "true"),
                ("llm.idle_minutes", "15"),
                ("ui.theme", "\"light\""),
            ] {
                c.execute(
                    "INSERT INTO main.settings (key,value_json,updated_at) VALUES (?1,?2,0)",
                    [k, v],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    }
    let h = harness_in(dir.clone());
    let users = h.svc.list().await.unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, migrate::FIRST_USER_NAME);
    assert!(!legacy.exists() && dir.join(migrate::LEGACY_RENAMED).exists());

    // 端末共通のキーは device_settings へ移り、どのユーザーでも読める
    assert_eq!(
        h.db.call_device(|c| settings::get_comfyui_url(c))
            .await
            .unwrap(),
        "http://10.0.0.5:8188"
    );
    h.svc.login(users[0].id.clone(), None).await.unwrap();
    let s = h.db.call(|c| settings::load(c)).await.unwrap();
    assert_eq!(s.theme, settings::Theme::Light);
    // 画像は移動せず、保存先は従来の位置に固定される
    assert_eq!(
        s.images_dir.as_deref(),
        Some(dir.join("images").to_str().unwrap())
    );
    assert_eq!(preset_count(&h).await, 1);
    let leftover: i64 = h
        .db
        .call(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM main.settings WHERE key IN ('debug.enabled','engine.comfyui.url','llm.idle_minutes')",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(leftover, 0);
    h.svc.logout().unwrap();
    drop(h);

    // 2 回目の起動では再実行されない
    let h2 = harness_in(dir);
    assert_eq!(h2.svc.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn fresh_install_starts_with_no_users() {
    let h = harness();
    assert!(h.svc.list().await.unwrap().is_empty());
}
