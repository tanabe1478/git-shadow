# git-shadow 使い方ガイド

> **[English version](usage.md)**

## インストール

```bash
# ビルドとインストール
cargo install --path .

# 確認
git-shadow --help
```

## セットアップ

リポジトリごとに一度 `install` を実行します:

```bash
cd your-repo
git-shadow install
```

以下が作成されます:
- `.git/shadow/` ディレクトリ (baselines, stash, config)
- Git hooks: `pre-commit`, `post-commit`, `post-merge`

既存の hook がある場合は `<hook>.pre-shadow` にリネームされ、git-shadow の処理後にチェーン実行されます。

## ファイルの管理

### Overlay: トラッキング済みファイルへのローカル変更

チームが既にトラッキングしているファイルに個人的な内容を追記したい場合に使います。

```bash
# トラッキング済みファイルを登録
git-shadow add docker-compose.yml

# 自由に編集 — あなたの変更は「shadow 変更」になる
echo "  # 個人用デバッグポート" >> docker-compose.yml
```

**コミット時の動作:**
1. あなたの追記が退避される
2. 元の内容（ベースライン）がコミットされる
3. コミット直後にあなたの追記が復元される

**オプション:**
- `--force` — 1MB のファイルサイズ上限をスキップ

### Phantom: ローカル限定ファイル

自分のマシンだけに存在するファイルを管理したい場合に使います。

```bash
# 新しいローカル限定ファイルを作成して登録
echo "#!/bin/bash" > scripts/local-setup.sh
git-shadow add --phantom scripts/local-setup.sh
```

デフォルトでは `.git/info/exclude` に追加され、`git status` に表示されなくなります。

**オプション:**
- `--no-exclude` — `.git/info/exclude` への追加をスキップ。`git status` には未追跡ファイルとして表示されますが、pre-commit hook によりコミットからは除外されます。

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

## 状態の確認と差分表示

### Status

```bash
git-shadow status
```

管理対象ファイルの情報を表示:
- Overlay: ベースラインのコミットハッシュ、差分行数 (+/- 行)
- Phantom: exclude モード、ファイルサイズ
- stale lock、stash 残留、ベースラインずれの警告

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

overlay をかけているファイルがチームによって更新された場合（`git pull` 後など）:

```bash
# post-merge hook が警告を表示:
# "warning: baseline for docker-compose.yml is outdated. Run `git-shadow rebase docker-compose.yml`"

# ベースラインを更新し shadow 変更を再適用
git-shadow rebase docker-compose.yml
```

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
- `git-shadow status` は "SUSPENDED" 状態を表示します
- `git-shadow doctor` は suspended 状態を警告として報告します

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

## 診断

```bash
git-shadow doctor
```

チェック項目:
- Hook ファイルの存在、実行権限、内容
- 競合する hook マネージャーの検出 (Husky, pre-commit, lefthook)
- config の整合性（管理対象ファイルとベースラインの存在確認）
- stash 残留や stale lock の有無

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

### パスのエンコーディング

ネストしたパスはフラットに保存するため URL エンコードされます:
- `scripts/local-setup.sh` → `scripts%2Flocal-setup.sh`
- `docs/100%done.md` → `docs%2F100%25done.md`

エンコード順序: `%` → `%25` を先に、次に `/` → `%2F`。

## 注意事項

### `git commit --no-verify`

`--no-verify` を使うと pre-commit hook がスキップされるため、shadow 変更がコミットに含まれます。これは Git の仕様上回避できません。shadow 管理対象ファイルに変更がある場合は `--no-verify` の使用を避けてください。

### 部分ステージ

git-shadow は overlay ファイルの部分ステージ (`git add -p`) をサポートしていません。overlay ファイルにステージ済みと未ステージの変更が同時に存在する場合、pre-commit hook がコミットをブロックします。コミット前に `git add <file>` でファイル全体をステージしてください。

### バイナリファイル

テキストファイルのみサポートしています。rebase コマンドがテキストベースの 3-way merge に依存しているため、バイナリファイルは `git-shadow add` 時に拒否されます。
