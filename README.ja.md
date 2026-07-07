# git-shadow

> **[English version](README.md)**

Git リポジトリ内の**ローカル限定の変更**を管理する CLI ツールです。開発中はワーキングツリーに変更が反映された状態で作業でき、コミット時には自動的に剥がされるため、Git の履歴がクリーンに保たれます。

## なぜ必要？

共有ファイルに個人的な変更を加えたいことがあります — デバッグ設定、ローカル環境のオーバーライド、個人的なメモなど。git-shadow を使えば、それらのローカル編集をチームのコミット履歴に残さずに管理できます。

## コンセプト

| 種別 | 説明 | 例 |
|------|------|-----|
| **overlay** | 既存のトラッキング済みファイルにローカル変更を重ねる | 共有の `docker-compose.yml` に個人用デバッグ設定を追加 |
| **phantom** | リポジトリに存在しないファイルをローカルだけで作成する | `scripts/local-setup.sh` をローカル限定で作成 |
| **phantom dir** | ディレクトリ全体をローカル限定で管理する（exclude のみ、stash/restore なし） | ローカル限定の `.claude/` ディレクトリを常にコミット対象外にする |

## インストール

### ビルド済みバイナリのダウンロード

お使いのプラットフォーム向けのバイナリを [GitHub Releases](https://github.com/tanabe1478/git-shadow/releases/latest) からダウンロードできます:

| プラットフォーム | アーキテクチャ | ダウンロード |
|----------|-------------|----------|
| Linux | x86_64 | [git-shadow-x86_64-unknown-linux-gnu.tar.gz](https://github.com/tanabe1478/git-shadow/releases/latest/download/git-shadow-x86_64-unknown-linux-gnu.tar.gz) |
| Linux | aarch64 | [git-shadow-aarch64-unknown-linux-gnu.tar.gz](https://github.com/tanabe1478/git-shadow/releases/latest/download/git-shadow-aarch64-unknown-linux-gnu.tar.gz) |
| macOS | Apple Silicon | [git-shadow-aarch64-apple-darwin.tar.gz](https://github.com/tanabe1478/git-shadow/releases/latest/download/git-shadow-aarch64-apple-darwin.tar.gz) |
| macOS | Intel | [git-shadow-x86_64-apple-darwin.tar.gz](https://github.com/tanabe1478/git-shadow/releases/latest/download/git-shadow-x86_64-apple-darwin.tar.gz) |

```bash
# 例: macOS Apple Silicon
curl -LO https://github.com/tanabe1478/git-shadow/releases/latest/download/git-shadow-aarch64-apple-darwin.tar.gz
tar xzf git-shadow-aarch64-apple-darwin.tar.gz
sudo mv git-shadow /usr/local/bin/
```

### ソースからビルド

```bash
cargo install --path .
```

## クイックスタート

```bash
# リポジトリで初期化
cd your-repo
git-shadow install

# 管理対象を追加（tracked は overlay、existing untracked は phantom を自動判定）
git-shadow add docker-compose.yml
git-shadow add scripts/local-setup.sh
echo "  # 個人用デバッグポート" >> docker-compose.yml

# 明示的に phantom / overlay を強制したい場合はフラグも使える
git-shadow add --phantom another-local-file.sh

# Git の状態と shadow の意味をまとめて確認
git shadow status --git

# 普通にコミット — shadow 変更は自動的に除外される
git add -A && git commit -m "チームの変更"

# 確認: 個人的な変更はワーキングツリーに残っている
cat docker-compose.yml        # 個人の追記あり
git show HEAD:docker-compose.yml  # クリーンなチーム用の内容のみ
```

## コマンド一覧

| コマンド | 説明 |
|---------|------|
| `git-shadow install` | Git hooks のセットアップ (pre-commit, post-commit, post-merge, post-rewrite)。`core.hooksPath` を尊重 |
| `git-shadow uninstall [--force]` | hooks・exclude・state を削除。`--force` は管理対象が残っていても overlay を復元して削除 |
| `git-shadow add <file>...` | tracked は overlay、既存の untracked path は phantom として自動登録 |
| `git-shadow add --phantom <file>...` | ローカル限定ファイル/ディレクトリを phantom として強制登録 |
| `git-shadow remove <file>` | shadow 管理から解除 |
| `git-shadow status [--git] [--json]` | 管理対象ファイルの一覧と状態を表示。`--git` で `git status --short --branch` も先に表示。`--json` はスクリプト向けの安定した英語 JSON を出力 |
| `git-shadow diff [file]` | shadow 変更の差分を表示 |
| `git-shadow rebase [file]` | ベースラインを更新し shadow 変更を再適用 (3-way merge) |
| `git-shadow restore [file]` | 中断されたコミットやクラッシュからの復旧 |
| `git-shadow suspend` | ブランチ切替のために shadow 変更を一時退避 |
| `git-shadow resume` | 退避した shadow 変更を復元（必要に応じて 3-way merge） |
| `git-shadow export [path] [--force]` | 管理中の state をポータブルな archive にまとめて別マシンへ移行 |
| `git-shadow import <archive> [--force]` | 新しく clone したリポジトリに archive から state を復元（3-way merge・デフォルト安全側） |
| `git-shadow doctor [--json]` | hooks・設定の整合性・残留状態を診断。問題があれば非ゼロで終了。`--json` は安定した英語 JSON を出力 |

`git-shadow --version` でインストール済みのバージョンを表示します。

## 仕組み

1. **pre-commit hook**: shadow 変更を退避し、ベースラインを復元してインデックスを更新
2. **git commit**: クリーンなベースライン（shadow 変更なし）を記録
3. **post-commit hook**: 退避していた shadow 変更をワーキングツリーに復元

すべてのデータは `.git/shadow/` に保存されます。`.git/` 内にあるため自動的にコミット対象外です。

**worktree 対応**: `git worktree` 環境では、hooks と exclude ルールはワークツリー間で共有されますが、shadow の状態（config, baselines, stash）はワークツリーごとに独立しています。各ワークツリーで `git-shadow install` を実行してください。メインリポジトリに shadow 管理対象ファイルがある場合、`install` 時に自動的にファイルリストが継承されます — overlay のベースラインはワークツリーの HEAD から再生成され、phantom エントリはそのままコピーされます。つまり、ワークツリーのセットアップは `install` コマンド一つで完了します。

## 安全性

- **原子的書き込み**: 一時ファイル → rename パターンでデータ破損を防止
- **ロックファイル**: PID ベースのロックで並行操作を防止
- **ロールバック**: pre-commit の失敗時は自動的にロールバック
- **リカバリ**: `git-shadow restore` であらゆる中断状態から復旧可能
- **自動回復**: stale lock は安全に直せる場合だけ自動回復し、上書きリスクがある場合は手動復旧を要求

## 日常運用のメモ

- デフォルトでは `git status` 自体は置き換えません。opt-in の統合表示として `git shadow status --git` を使ってください。
- Git には一般的な pre-`add` hook がないため、overlay への早期警告は `git-shadow status` と commit 時の警告で補います。

## Claude Code プラグイン

このリポジトリには、AI コーディングエージェントが `git-shadow` を正しく扱えるようにするための
[Claude Code](https://code.claude.com) スキルが同梱されています（[`skills/git-shadow/SKILL.md`](skills/git-shadow/SKILL.md)）。
リポジトリ自体がプラグインの marketplace になっています。

**marketplace 経由でインストール（推奨）:**

```text
/plugin marketplace add tanabe1478/git-shadow
/plugin install git-shadow@git-shadow
```

**プラグインを使わない環境向けのフォールバック** — スキルを個人の skills ディレクトリに
コピーまたはシンボリックリンクします:

```bash
ln -s "$(pwd)/skills/git-shadow" ~/.claude/skills/git-shadow   # または: cp -r skills/git-shadow ~/.claude/skills/git-shadow
```

## ドキュメント

- [詳細な使い方ガイド](docs/usage.ja.md) | [English](docs/usage.md)
- [要件定義](docs/requirements.md)

## 動作要件

- Git 2.20+（worktree の完全サポートには Git 2.31+ を推奨）
- Rust 1.70+（ソースからビルドする場合のみ）

## ライセンス

MIT
