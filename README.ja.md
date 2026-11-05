# tmux Claude Monitor

[English README here](README.md)

メニューバー常駐の召喚型パネルで、tmux 内で動いているすべての Claude Code セッションを一覧監視する macOS アプリ（Tauri v2）。

- **⌘⇧K で召喚/非表示**（グローバルショートカット、`settings.json` で変更可・変更は再起動後に反映）
- 各カード: 状態ドット（🟢作業中 / 🟡入力待ち / ⚪アイドル・放置）・タイトル・直近アクティビティの要約・アイドル時間
- 入力待ち（権限ダイアログ等の structured prompt）になった瞬間に macOS 通知。文末が「?」のただの質問はアンバー表示のみで通知しない
- **ジャンプ（Enter でもクリックでも）すると iTerm2 を前面化して該当ペインに切り替え、パネルは自動で隠れる**
- 全 Space・フルスクリーンアプリの上にも表示（NSWindow collectionBehavior shim）
- **バックグラウンド常駐**: 起動時は非表示（メニューバーのアイコンが起動確認）。ウィンドウを閉じても終了しない。終了はトレイメニューの Quit から。入力待ちセッションがあるとトレイに `● N` 表示
- ログイン時に自動起動したい場合: システム設定 > 一般 > ログイン項目 にアプリを追加

## キー操作（vim 風モーダル）

| キー | モード | 動作 |
|---|---|---|
| `⌘⇧K` | どこでも | パネルの表示/非表示トグル |
| `j` / `k` (`↓`/`↑`) | normal | 選択を下/上へ |
| `g` / `G` | normal | 先頭 / 末尾へ |
| `/` | normal | 検索モードへ（ファジー検索、マッチ文字はハイライト） |
| `Enter` | 両方 | 選択セッションへジャンプしてパネルを隠す |
| `Esc` | normal | パネルを隠す |
| `Esc` | search | クエリをクリアして normal へ |
| `Backspace`（空クエリ時） | search | normal へ戻る |
| `^j` / `^k` (`↓`/`↑`) | search | 入力しながら選択移動 |

## Build & Install

```bash
pnpm install
pnpm tauri build          # → src-tauri/target/release/bundle/macos/tmux-claude-monitor.app
cp -R src-tauri/target/release/bundle/macos/tmux-claude-monitor.app /Applications/
```

通知はバンドルされた .app として起動した場合のみ機能する（`tauri dev` では出ない）。署名はアドホックで良い（個人利用）。

## 初回起動時の権限

1. **通知** — 最初の通知時に macOS が許可を求める → 許可
2. **オートメーション (iTerm2)** — 最初にカードをクリックした時に
   「"tmux-claude-monitor" wants access to control "iTerm2"」→ 許可
   （システム設定 > プライバシーとセキュリティ > オートメーション で後から変更可）

## 設定

ヘッダーの ⚙ をクリックすると `settings.json` がエディタで開く。編集は再起動なしで次のポーリングから反映される。

| key | default | 意味 |
|---|---|---|
| `poll_interval_ms` | 2000 | ポーリング間隔（下限 250ms にクランプ） |
| `stale_after_min` | 30 | この分数アイドルで「放置」(gray) 扱い |
| `notifications` | true | macOS 通知の有効/無効 |
| `window_pos` | null | パネル位置（ドラッグで自動保存） |
| `global_shortcut` | "cmd+shift+k" | 召喚ショートカット（**再起動後に反映**。競合で登録に失敗した場合はトレイメニューに ⚠ 表示） |

場所: `~/Library/Application Support/jp.isa.tmux-claude-monitor/settings.json`

## 仕組み（概要）

- 2秒ごとに `tmux list-panes -a` → 各ペインのプロセスツリー（`ps` スナップショット1回）から claude プロセスを検出
- タイトル/要約は `~/.claude/projects/<escaped-cwd>/<session>.jsonl` の末尾 256KB と、ペイン上部のルーラータイトルから導出
- 状態は `tmux capture-pane` のヒューリスティクス（`esc to interrupt` = 作業中、`❯ 1.` / `Enter to confirm` = 入力待ち（`Do you want` は意図的にマーカーにしない — 通常の文章ターンの末尾に頻出するため））。パターンは `src-tauri/src/state_detect.rs` に集約、実画面フィクスチャ（`src-tauri/tests/fixtures/`）でテスト
- 観測は読み取り専用。tmux への書き込みはクリック時の `switch-client` のみ

### 既知の制限 (v1)

- 同じリポジトリで `--resume` なしの claude が複数動いていると、要約が最新 mtime の transcript に揃ってしまう（タイトルはペイン由来なので正しい）
- iTerm2 専用ジャンプ。tmux クライアントが未アタッチの場合はエラートースト表示のみ
- 文中に埋もれた質問（文末が `?` でない）は入力待ちとして検出されない

## Development

```bash
pnpm tauri dev                  # dev app
pnpm test                       # frontend unit tests (vitest)
cd src-tauri && cargo test      # Rust unit tests (fixtures included)
cd src-tauri && cargo run --example scan   # 1 tick をCLIで実行して検出結果を表示
```

設計ドキュメント: `docs/superpowers/specs/2026-11-05-tmux-claude-monitor-design.md`
実装計画: `docs/superpowers/plans/2026-11-05-tmux-claude-monitor.md`
