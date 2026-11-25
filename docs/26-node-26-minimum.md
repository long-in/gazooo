# 26. Node.js の最低要件を 26 に引き上げる(v0.18)

> ステータス: 🟢 承認済み・実装済み(2026-11-20。07 Q-129〜132 は推奨どおり。手元の Node 26 で lint・型チェック・Vitest 209 件・ビルドまで確認。GitHub Actions での Node 26 は初回実行が未確認)

## 1. 目的とスコープ

最低要件を Node.js 22 LTS から **Node.js 26 以上** に引き上げ、開発環境・CI・リリースを同じ Node に揃える。きっかけは、開発機の Node 26.13.1 で Vitest 5.1.1 の既定プールが全テスト失敗("Vitest failed to find the runner")したこと。VM 系プール(`vmThreads`)では 23 ファイル・209 テストが通ることを確認済み。Node 22 側での挙動は未確認。

スコープ外: アプリの機能変更。Rust・IPC・DB の変更は **なし**。

## 2. 変更内容

| 対象 | 変更 |
|---|---|
| `package.json` の `engines.node` | `>=22` → `>=26` |
| CI(`ci.yml` の 2 箇所)・リリース(`release.yml` の 2 箇所)の `node-version` | `22` → `26` |
| `@types/node` | 22.x → 26.x(最低要件に合わせる。02 §依存更新方針の「22.x のまま」を改める) |
| `vitest.config.ts` | Node 26 以上のときだけ `pool: "vmThreads"` に切り替える条件分岐をやめ、**常に `vmThreads`**(下記 §3) |
| 文書 | AGENTS.md §2、README、02 §依存更新方針、06 §共通・§CI の「Node 22 LTS」を「Node 26」に更新 |

`package-lock.json` は `@types/node` の更新に伴って更新し、コミットする(AGENTS.md「依存バージョンの固定」)。依存更新を目的とした変更として、`npm install @types/node@<version>` → 固定値を確認 → lint・テスト・ビルドで検証する。

## 3. Vitest の回避策

- 最低要件が 26 になるため、`vitest.config.ts` の `pool: "vmThreads"` は条件分岐なしで常に有効にする。
- 原因が Vitest 側か Node 側かは未特定。**Vitest の更新時に既定プールで通るか再確認し、通れば回避策を外す**(依存更新の検証項目に加える)。
- VM 系プールは長時間の実行でメモリが増えやすい。現状のテスト規模(209 件)では問題なし。

## 4. 検証

- 手元(Node 26)で `npm ci` → `npm run lint` → `npm run typecheck` → `npm test` → `npm run build`。
- GitHub Actions は Windows・macOS の両方で通ること(Node 26 は初回実行。`actions/setup-node` が 26 を取得できること、Tauri のビルドに影響が無いことを確認)。
- Node 22 / 24 での動作保証は **しない**(`engines` で弾く)。

## 5. リスク

- CI で Node 26 を取得できない、または Tauri のビルドツールが Node 26 に未対応。→ 初回の CI で確認し、失敗時は 24 LTS への変更を提案する。
- Node 26 を入れていない開発者は更新が必要。→ README と `engines` で明示する。

## 6. 未決事項(07 B17 に追記)

| ID | 論点 | 推奨 |
|---|---|---|
| Q-129 | 最低要件 | **Node 26 以上**(ユーザー指示。22 / 24 は対象外) |
| Q-130 | `@types/node` | **26.x に更新**(最低要件に合わせる) |
| Q-131 | Vitest のプール | **常に `vmThreads`**。Vitest 更新時に既定プールで通れば外す |
| Q-132 | CI の初回失敗時 | **24 LTS への変更を再提案**(勝手に下げない) |
