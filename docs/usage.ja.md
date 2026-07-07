# git-shadow 使い方ガイド

> **[English version](usage.md)**

## インストール

```bash
# ビルドとインストール
cargo install --path .

# 確認
git-shadow --help
git-shadow --version
```

## セットアップ

リポジトリごとに一度 `install` を実行します:

```bash
cd your-repo
git-shadow install
```

以下が作成されます:
- `.git/shadow/` ディレクトリ (baselines, stash, config)
- Git hooks: `pre-commit`, `post-commit`, `post-merge`, `post-rewrite`

既存の hook がある場合は `<hook>.pre-shadow` にリネームされ、git-shadow の処理後にチェーン実行されます。

> **`core.hooksPath`**: リポジトリで `core.hooksPath`（Husky、lefthook、独自の `dev-hooks/` など）が設定されている場合、`install` は hooks を実際に発火する有効なディレクトリに配置し、`note: core.hooksPath (.husky) is set, so hooks were installed into <path>` のようなメッセージを表示します。既定のディレクトリに hooks があるのに `core.hooksPath` が別を指している（＝発火せず黙って無視される）場合、`git-shadow doctor` が issue として報告します。

> **worktree**: `git worktree` を使用している場合は、各ワークツリーで `git-shadow install` を個別に実行してください。メインリポジトリに shadow 管理対象ファイルがある場合、`install` 時に自動的にファイルリストが継承されます（overlay のベースラインはワークツリーの HEAD から再生成、phantom エントリはそのままコピー）。詳細は [git worktree 対応](#git-worktree-対応) を参照してください。

## ファイルの管理

### ファイルの追加

`git-shadow add` は 1 つ以上のパスを受け取り、それぞれの管理方法を自動で判定します:

- **トラッキング済みファイル** は **overlay**（コミット済み内容に重ねるローカル変更）になります。
- **既存の未追跡パス** は **phantom**（自分のマシンだけに存在するファイルまたはディレクトリ）になります。

```bash
# 複数のファイルを一度に追加 — それぞれ自動で判定される
git-shadow add docker-compose.yml scripts/local-setup.sh .env.local
```

判定できないパス（トラッキングされておらず、ディスク上にも存在しない）があった場合、そのパスはエラーになり、残りのパスは処理が続行されます。1 つでも失敗すると、コマンドは非ゼロで終了します。

**オプション:**
- `--overlay` — 指定したパスをすべて overlay として強制登録（ファイルはトラッキング済みである必要があります）
- `--phantom` — 指定したパスをすべて phantom として強制登録（パスはトラッキングされていない必要があります）
- `--no-exclude` — `.git/info/exclude` への追加をスキップ（phantom のみ）。`git status` には未追跡ファイルとして表示されますが、pre-commit hook によりコミットからは除外されます。
- `--force` — overlay の 1MB ファイルサイズ上限を無視

`--overlay` と `--phantom` は同時に指定できません。

### Overlay: トラッキング済みファイルへのローカル変更

チームが既にトラッキングしているファイルに個人的な内容を追記したい場合に使います。

```bash
# トラッキング済みファイルを登録（overlay として自動判定）
git-shadow add docker-compose.yml

# 自由に編集 — あなたの変更は「shadow 変更」になる
echo "  # 個人用デバッグポート" >> docker-compose.yml
```

**コミット時の動作:**
1. あなたの追記が退避される
2. 元の内容（ベースライン）がコミットされる
3. コミット直後にあなたの追記が復元される

### Phantom: ローカル限定ファイル

自分のマシンだけに存在するファイルを管理したい場合に使います。

```bash
# 新しいローカル限定ファイルを作成して登録（phantom として自動判定）
echo "#!/bin/bash" > scripts/local-setup.sh
git-shadow add scripts/local-setup.sh
```

デフォルトでは `.git/info/exclude` に追加され、`git status` に表示されなくなります。`--no-exclude` でこの追加をスキップできます。

#### Phantom ディレクトリ

ディレクトリ全体を phantom として登録することもできます:

```bash
# ローカル限定ディレクトリを登録
git-shadow add --phantom .claude/
git-shadow add --phantom codemaps/
```

ディレクトリ phantom は `.git/info/exclude` による管理のみ行われ、stash/restore は不要です。ディレクトリとその中身はワーキングツリーに常に残り、誤って `git add` されたファイルは pre-commit hook で自動的にアンステージされます。

`git-shadow status` ではディレクトリ phantom は `(phantom dir)` ラベルとエントリ数で表示されます。

### 管理の解除

```bash
git-shadow remove docker-compose.yml
```

- **Overlay**: ファイルをベースラインの内容に戻します。shadow 変更は破棄されます。
- **Phantom**: ファイルはディスクに残りますが、管理対象から外れます。`.git/info/exclude` のエントリも削除されます。

解除前に確認プロンプトが表示されます。`--force` でスキップできます（非対話環境では必須）。

## アンインストール

リポジトリから git-shadow を完全に削除するには:

```bash
git-shadow uninstall
```

以下の処理が行われます:
- 有効な hooks ディレクトリ（`core.hooksPath` を尊重）から git-shadow の hooks を削除し、install 時に退避した `<hook>.pre-shadow` backup を復元します
- このワークツリーが所有する `.git/info/exclude` の管理セクションのエントリを削除します（他のワークツリーが所有するエントリは保持されます）
- このワークツリーの shadow state（`.git/shadow/`）を削除します

安全のため、`uninstall` は次の 2 つの状況では実行を**拒否**します:
- **管理対象ファイルが残っている** — 件数を示すエラーで停止します。`git-shadow remove <file>` で個別に解除するか、`--force` を付けて再実行してください。
- **コミットが進行中** — stash の残骸や、別の live プロセスが保持している lock があると、commit サイクルが進行中と判断され、state を消すと作業を失う可能性があります。

```bash
# 管理対象が残っていても overlay を復元して state を削除する
git-shadow uninstall --force
```

`--force` を付けると、overlay ファイルはベースラインの内容に復元され（shadow 変更は破棄）、件数が `restored baselines to the working tree for 1 overlay(s)` のように報告されます。phantom ファイルはあなたのローカル限定ファイルなので、ディスク上でそのまま残されます。成功時には `git-shadow uninstalled (hooks, exclude entries, and state removed)` と表示されます。

### 手動での削除

バイナリが使えず、手作業で git-shadow を削除する必要がある場合:

1. 有効な hooks ディレクトリ（`.git/hooks/` または `core.hooksPath`）で、`git-shadow hook` を呼び出す `pre-commit`・`post-commit`・`post-merge`・`post-rewrite` スクリプトを削除します。`<hook>.pre-shadow` backup があれば `<hook>` に戻します。
2. コミット済み内容に戻したい overlay ファイルを復元します（例: `git restore --source=HEAD -- <file>`）。
3. `.git/shadow/` を削除します。
4. `.git/info/exclude` から git-shadow の管理セクション（マーカーコメントで囲まれた範囲）を削除します。

## 状態の確認と差分表示

### Status

```bash
git-shadow status
```

管理対象ファイルの情報を表示:
- Overlay: ベースラインのコミットハッシュ、差分行数 (+/- 行)
- Overlay: 現在の Git 状態 (`clean`, `modified`, `staged`, `partially staged`)
- Overlay: stage 済み local-only changes が commit 前に取り除かれる警告
- Phantom: exclude モード、ファイルサイズ
- stale lock、stash 残留、ベースラインずれの警告

通常の Git 出力も含めた opt-in 表示を使いたい場合:

```bash
git shadow status --git
```

`git status --short --branch` を先に表示し、その後に shadow 管理対象の意味づけを表示します。デフォルトでは `git status` 自体は置き換えません。

スクリプトで使う場合は `--json` を指定します:

```bash
git-shadow status --json
```

安定した英語（非ローカライズ）の JSON を出力し、人間向けの出力は抑制されます。キーはパース向けの安定した識別子です。例えば `git_state` は `clean`・`modified`・`staged`・`partially_staged` のいずれか、`warnings` には `stash_remaining` や `stale_lock` などのトークンが入ります:

```json
{
  "suspended": false,
  "warnings": [],
  "files": [
    {
      "path": "docker-compose.yml",
      "type": "overlay",
      "exists": true,
      "baseline_commit": "f5fb751...",
      "shadow_added": 1,
      "shadow_removed": 0,
      "git_state": "modified",
      "baseline_outdated": false
    }
  ]
}
```

### Diff

```bash
# すべての shadow 変更を表示
git-shadow diff

# 特定ファイルの変更を表示
git-shadow diff docker-compose.yml
```

- **Overlay**: ベースラインと現在の内容のカラー unified diff を表示
- **Phantom**: ファイル全体を新規ファイル diff として表示

## アップストリームの変更への対応

overlay をかけているファイルがチームによって更新された場合（`git pull` 後など）、`post-merge` と `post-rewrite` hook が自動的に実行されます:

- **クリーンなマージ** — ベースラインと shadow 変更が自動的に再適用されます（クリーン時限定の自動 rebase）。操作は不要です。
- **コンフリクト** — 自動 rebase はスキップされ、`git-shadow rebase <file>` で手動解決するよう警告されます。

手動 rebase が必要な場合は明示的に実行します:

```bash
# ベースラインを更新し shadow 変更を再適用
git-shadow rebase docker-compose.yml
```

hook 実行時に別の live プロセスが lock を保持している場合、安全のため自動 rebase はスキップされます。後から自分で `git-shadow rebase` を実行してください。

rebase は 3-way merge を実行します:
1. 旧ベースライン（共通祖先）
2. 現在の内容（shadow 変更込み）
3. 新しい HEAD の内容（アップストリームの変更）

コンフリクトが発生した場合は、標準的なコンフリクトマーカー (`<<<<<<<`, `=======`, `>>>>>>>`) がファイルに書き込まれます。

```bash
# すべての overlay ファイルを一括で rebase
git-shadow rebase
```

## ブランチ切替

overlay の変更はワーキングツリーを変更するため、`git checkout` がブロックされることがあります。`suspend` と `resume` を使ってクリーンにブランチを切り替えられます。

### Suspend

```bash
# shadow 変更を退避してベースラインを復元
git-shadow suspend
```

以下の処理が行われます:
1. 各 overlay のワーキングツリーの内容を `.git/shadow/suspended/` に保存
2. ベースラインの内容をワーキングツリーに復元
3. 各 phantom ファイルを `.git/shadow/suspended/` に保存し、ワーキングツリーから削除
4. config を "suspended" 状態に設定

ワーキングツリーがクリーンになるので、自由にブランチを切り替えられます。

### Resume

```bash
# ブランチ切替後、shadow 変更を復元
git-shadow resume
```

ベースラインが変わっていない場合（同じブランチ、またはファイル内容が同一）は、退避した内容がそのまま復元されます。ベースラインが変わっている場合（別ブランチ）は、3-way merge が実行されます:

1. 旧ベースライン（suspend 前のもの）
2. 退避した内容（あなたの shadow 変更）
3. 新しい HEAD の内容（現在のブランチのバージョン）

コンフリクトが発生した場合は、標準的なコンフリクトマーカーが書き込まれます。

### 典型的なワークフロー

```bash
# feature ブランチで shadow 変更を加えて作業中
git-shadow suspend
git checkout main
git-shadow resume          # main の内容に shadow 変更を再適用

# 元のブランチに戻る
git-shadow suspend
git checkout feature
git-shadow resume          # shadow 変更を復元
```

### Suspended 中の制限事項

- `git commit` はブロックされます（pre-commit hook がエラーを返す）
- `git-shadow diff` と `git-shadow rebase` はブロックされます
- `git-shadow export` はブロックされます（先に resume すれば archive に shadow の内容が入ります）
- `git-shadow status` は "SUSPENDED" 状態を表示します
- `git-shadow doctor` は suspended 状態を警告として報告します

## 新しいマシンへの移行

shadow の state は `.git/shadow/` と `.git/info/exclude` に保存されるため、`git clone` では引き継がれません。別マシンで新しく clone したリポジトリにローカル専用の設定を移すには、`export` / `import` を使います。

> ディスク全体を移行する（あるいは `.git` ディレクトリごとコピーする）場合は、shadow の state もそのまま付いてくるため export/import は不要です。`export` / `import` は「新しく clone した」よくあるケース向けです。

### Export

```bash
# 管理中の state をポータブルな archive にまとめる
git-shadow export

# → exported 2 managed file(s) to `/path/to/git-shadow-export.tar.gz`

# 出力先を明示することもできる
git-shadow export ~/backups/shadow.tar.gz

# 既存の archive を上書きする
git-shadow export --force ~/backups/shadow.tar.gz
```

archive は gzip 圧縮された tar (`.tar.gz`) で、`manifest.json`（`format_version`・ツールバージョン）と管理対象ごとの内容ファイルを含みます。含まれる内容:

- **overlay** — 現在の working tree（shadow）の内容 **と** 保存済み baseline **と** baseline commit。移行先リポジトリが先に進んでいた場合に `import` が 3-way merge できるようにするためです。
- **phantom file** — ファイルの内容。
- **phantom directory** — 配下の全ファイル（再帰的）。

バイナリ内容は生バイトとして保存され、バイト単位で完全に往復します。export は、管理対象が無いとき・suspend 中（先に `git-shadow resume`）・commit 処理の途中（stash 残骸 / ライブロック）には実行を拒否します。既定の出力先はカレントディレクトリの `git-shadow-export.tar.gz` で、既存ファイルは `--force` なしでは上書きしません。

### Import

新しいマシンでリポジトリを clone し、`git-shadow install` を実行してから import します:

```bash
git clone <repo-url> myrepo
cd myrepo
git-shadow install
git-shadow import /path/to/git-shadow-export.tar.gz

# → config.txt: imported overlay
#   notes.md: imported phantom
#   import finished: 2 imported, 0 skipped
```

import は **デフォルトで安全側** に動作し、問題があっても続行して最後に集計を報告します:

- **phantom file / directory** — パスが存在しない、または既存でも内容が同一（冪等）なら書き込んで登録します。**異なる**内容で既に存在する場合は、ファイルごとのメッセージを出してスキップし、コマンドは非ゼロで終了します。`--force` を付けると上書きします。
- **overlay** — 対象は HEAD に追跡されている必要があります（そうでなければスキップ: リポジトリが export 元と一致しません）。baseline は現在の HEAD から再生成します。HEAD が archive 内の baseline と一致すれば shadow の内容をそのまま書き込み、異なれば 3-way merge（archive の baseline / archive の shadow / 現在の HEAD）で shadow の変更を upstream の上に再適用します。クリーンな merge は書き込み、**conflict** はスキップ（conflict marker は書き込みません）してコマンドは非ゼロ終了します。`--force` を付けると、衝突する upstream の hunk を捨てて shadow を優先します。
- **管理済みエントリ** — 同一内容の再 import は no-op。別の種類として管理されているエントリは、`--force` で置き換える場合を除きスキップします。

import は conflict を越えて続行するため、**解消してから再実行**すれば（衝突するローカル変更を消す、または `--force` を付ける）残りのエントリを処理できます。import は先に `git-shadow install` が必要で、suspend 中や commit 処理の途中では拒否します。また manifest の `format_version` を検証し、未知のバージョンには明確なエラーを返します。

## リカバリ

### 自動検出

コミットが中断された場合（エディタを閉じた、commit-msg hook の失敗など）、shadow 変更は退避されたまま復元されません。次回の git-shadow コマンド実行時に検出されます:

```
warning: stash has remaining files (a previous commit may have been interrupted)
  -> Run `git-shadow restore`
```

### 手動リカバリ

```bash
# すべての退避ファイルを復元し、ロックをクリーンアップ
git-shadow restore

# 特定ファイルを復元
git-shadow restore docker-compose.yml
```

`restore` はあらゆる異常状態に対応します:
- 退避ファイルをワーキングツリーに復元
- stale lockfile を削除
- stash ディレクトリをクリーンアップ

`git commit` 中に stale lock が見つかった場合も、作業ツリーを安全に復元できると判断できるときは自動回復を試みます。新しい内容を上書きする恐れがある場合だけ、従来どおり手動 `git-shadow restore` が必要です。

`restore` は、別の **live** プロセスが lock を保持している場合（実際の commit や hook が進行中の場合）は実行を拒否するため、他のプロセスの作業を上書きすることはありません。所有プロセスが既に存在しない（stale な）lock だけをクリーンアップします。

### トラブルシューティング

| 症状 | 原因 | 対処 |
|------|------|------|
| shadow 変更がコミットされる / hooks が実行されない | `core.hooksPath` が hooks の場所とは別を指しており、hooks が発火しない | `git-shadow install` を再実行する（有効な hooks ディレクトリにインストールされます）。`git-shadow doctor` が issue として報告します。 |
| `git commit` が "another git-shadow process still holds the lock" でブロックされる | 別の live な commit / hook が実行中 | 終わるまで待つ。実際には何も動いていないなら lock は stale なので `git-shadow restore` を実行する。 |
| `git commit` が "leftover files remain in `.git/shadow/stash/`" でブロックされる | 前回の commit が中断された | `git-shadow restore` を実行してから、もう一度 commit する。 |
| `git-shadow restore` が実行を拒否する | live プロセスが lock を保持している | そのプロセスの終了を待つ。restore は stale な lock だけを片付けます。 |
| `git-shadow resume` が "was edited in the working tree while suspended" でブロックされる | suspend 後にファイルを編集したため、resume すると編集が上書きされる | ファイルを確認し、残したい内容を退避し、`.git/shadow/suspended/` の内容と統合してから、もう一度 `git-shadow resume` を実行する。 |
| `git-shadow doctor` が非ゼロで終了する | 1 件以上の issue を検出した（壊れた hooks、baseline の欠落、inert な hooks など） | `issues:` のリストを読み、それぞれ対処する。warning だけなら非ゼロにはなりません。 |

## 診断

```bash
git-shadow doctor
```

チェック項目:
- Hook ファイルの存在、実行権限、内容、および inert でないこと（既定のディレクトリに hooks があるのに `core.hooksPath` が別を指していないか）
- 競合する hook マネージャーの検出 (Husky, pre-commit, lefthook)
- config の整合性（管理対象ファイルとベースラインの存在確認）
- stash 残留や stale lock の有無
- suspend 状態、worktree の初期化

検出結果は **issues**（赤 `✗`、壊れているもの）と **warnings**（黄 `⚠`、注意が必要なもの）に分かれます。

**終了コード:** `doctor` は 1 件以上の issue を検出すると非ゼロで終了します（例: `Error: doctor found 4 issue(s)`）。これによりスクリプトや CI で判定に使えます。warning だけの場合は終了コード 0 のままです。

スクリプトで使う場合は `--json` を指定します:

```bash
git-shadow doctor --json
```

安定した英語（非ローカライズ）の JSON を出力し、人間向けの出力は抑制されます。`ok` フィールドは issue がある場合に `false` になります（非ゼロの終了コードと一致します）:

```json
{
  "ok": true,
  "issues": [],
  "warnings": []
}
```

## データ保存先

すべてのデータは `.git/shadow/` 内に保存されます。`.git/` 内にあるため自動的にコミット対象外です:

```
.git/shadow/
├── config.json          # 管理対象ファイルのリスト・メタデータ
├── lock                 # PID ベースのロックファイル
├── baselines/           # ベースラインのスナップショット (URL エンコードされたファイル名)
│   └── docker-compose.yml
│   └── scripts%2Flocal-setup.sh
├── stash/               # コミット中の一時退避先
│   └── ...
└── suspended/           # suspend 時に退避した shadow 変更（ブランチ切替用）
    └── ...
```

`git worktree` 環境では、ストレージは以下の 2 つのディレクトリに分かれます:

| 場所 | スコープ | 内容 |
|------|---------|------|
| `git_dir`（ワークツリーごとの `.git`） | ワークツリー固有 | `shadow/`（config, baselines, stash, suspended, lock） |
| `common_dir`（共有 `.git`） | 全ワークツリー共有 | `hooks/`, `info/exclude` |

各ワークツリーは独立した shadow 状態を持ち、hooks と exclude ルールはすべてのワークツリーで共有されます。

### パスのエンコーディング

ネストしたパスはフラットに保存するため URL エンコードされます:
- `scripts/local-setup.sh` → `scripts%2Flocal-setup.sh`
- `docs/100%done.md` → `docs%2F100%25done.md`

エンコード順序: `%` → `%25` を先に、次に `/` → `%2F`。

## ワークフロー

### 基本: 単一リポジトリでのセットアップ

```bash
git-shadow install
git-shadow add docker-compose.yml     # overlay: Git追跡済みファイルのローカル上書き
git-shadow add --phantom .env.local  # phantom: ローカル限定の設定（未追跡）

# 通常の開発 — shadow の変更は自動的にコミットから除外される
vim docker-compose.yml
git commit -am "feat: add login"   # ローカルの上書きはコミットされない
```

### worktree の追加

worktree を作成したら `git-shadow install` を1回実行するだけで、メインリポの管理ファイルリストが自動的に継承されます。

```bash
git worktree add ../feature-branch feature/auth
cd ../feature-branch
git-shadow install
# → "inherited 2 file(s) from main worktree"
# → overlay のベースラインは HEAD から再生成
# → phantom エントリはそのままコピー

# すぐに作業開始できる
vim .env.local
git commit -am "feat: auth"        # shadow の変更は除外される
```

### worktree ごとのカスタマイズ

継承後、各 worktree で独立してファイルの追加・削除ができます。

```bash
cd ../feature-branch
git-shadow add --phantom TODO.md   # この worktree だけ
git-shadow remove notes.md         # この worktree だけ
```

### PR レビュー用の一時 worktree

```bash
git worktree add ../review-pr-42 pr/42
cd ../review-pr-42
git-shadow install                 # 設定を継承、すぐにビルド・テスト可能

# レビュー完了後、worktree を削除（shadow 状態も自動クリーンアップ）
cd ../main-repo
git worktree remove ../review-pr-42
```

### worktree を使わないブランチ切替

単一の作業ツリーでブランチを切り替える場合は、suspend/resume を使います。

```bash
git-shadow suspend                 # shadow の変更を退避
git checkout other-branch
git-shadow resume                  # 3-way merge で復元
```

worktree を使う場合、suspend/resume は不要です（各 worktree が独立した状態を持つため）。

### 操作の早見表

| やりたいこと | コマンド |
|---|---|
| 初回セットアップ | `git-shadow install` → `git-shadow add <file>` |
| worktree を追加して使う | `git worktree add ...` → `cd` → `git-shadow install` |
| worktree 固有のファイルを追加 | `git-shadow add --phantom <file>` |
| worktree を削除 | `git worktree remove <path>`（shadow 状態も消える） |
| 状態を確認 | `git-shadow status` / `git-shadow doctor` |
| ブランチ切替（worktree なし） | `git-shadow suspend` → checkout → `git-shadow resume` |
| リポジトリから git-shadow を削除 | `git-shadow uninstall`（または `--force`） |

## 注意事項

### `git commit --no-verify`

`--no-verify` を使うと pre-commit hook がスキップされるため、shadow 変更がコミットに含まれます。これは Git の仕様上回避できません。shadow 管理対象ファイルに変更がある場合は `--no-verify` の使用を避けてください。

### 部分ステージ

git-shadow は overlay ファイルの部分ステージ (`git add -p`) をサポートしていません。overlay ファイルにステージ済みと未ステージの変更が同時に存在する場合、pre-commit hook がコミットをブロックします。コミット前に `git add <file>` でファイル全体をステージしてください。

### `git add` 時のガード

Git には一般的な pre-`add` hook がないため、git-shadow はすべての `git add` の直前には警告できません。代わりに:

- `git-shadow status` で overlay が local-only かつ stage 済みかを表示します
- `git shadow status --git` で通常の Git 状態を含む opt-in 表示を使えます
- pre-commit hook が local-only な overlay 変更が取り除かれることを警告します

### バイナリファイル

テキストファイルのみサポートしています。rebase コマンドがテキストベースの 3-way merge に依存しているため、バイナリファイルは `git-shadow add` 時に拒否されます。

### git worktree 対応

git-shadow は `git worktree` 環境に対応しています。各ワークツリーは独立した shadow 環境として扱われます:

- **ワークツリー固有の状態**: config, baselines, stash, suspended 状態, lockfile は各ワークツリーの `.git` ディレクトリに保存されます。
- **install 時の自動継承**: ワークツリーで `git-shadow install` を実行すると、メインリポジトリに shadow 管理対象ファイルがあり、ワークツリーにまだ config が存在しない場合、ファイルリストが自動的に継承されます。overlay のベースラインはワークツリーの HEAD から再生成され、phantom エントリはそのままコピーされます。出力メッセージは `inherited N file(s) from main worktree` です。
- **共有リソース**: Git hooks と `.git/info/exclude` のエントリは共通の Git ディレクトリに保存され、すべてのワークツリーで共有されます。
- **診断**: `git-shadow doctor` はワークツリー内にいることを検出し、shadow が未初期化の場合は警告を表示します。
- **Git バージョン**: worktree の完全サポートには Git 2.31+ を推奨します（`--path-format=absolute` 対応）。古いバージョン（2.20+）もフォールバックで動作しますが、2.31+ が推奨です。

```bash
# メインリポジトリ
cd my-repo
git-shadow install
git-shadow add docker-compose.yml

# ワークツリーを作成してセットアップ — install が管理対象ファイルを自動継承
git worktree add ../my-repo-feature feature-branch
cd ../my-repo-feature
git-shadow install              # メインリポジトリから docker-compose.yml を継承
```
