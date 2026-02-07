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

## クイックスタート

```bash
# ソースからビルド
cargo install --path .

# リポジトリで初期化
cd your-repo
git-shadow install

# overlay を追加（既存のトラッキング済みファイル）
git-shadow add docker-compose.yml
echo "  # 個人用デバッグポート" >> docker-compose.yml

# phantom を追加（新規ローカル限定ファイル）
echo "#!/bin/bash" > scripts/local-setup.sh
git-shadow add --phantom scripts/local-setup.sh

# 普通にコミット — shadow 変更は自動的に除外される
git add -A && git commit -m "チームの変更"

# 確認: 個人的な変更はワーキングツリーに残っている
cat docker-compose.yml        # 個人の追記あり
git show HEAD:docker-compose.yml  # クリーンなチーム用の内容のみ
```

## コマンド一覧

| コマンド | 説明 |
|---------|------|
| `git-shadow install` | Git hooks のセットアップ (pre-commit, post-commit, post-merge) |
| `git-shadow add <file>` | トラッキング済みファイルを overlay として登録 |
| `git-shadow add --phantom <file>` | ローカル限定ファイルを phantom として登録 |
| `git-shadow remove <file>` | shadow 管理から解除 |
| `git-shadow status` | 管理対象ファイルの一覧と状態を表示 |
| `git-shadow diff [file]` | shadow 変更の差分を表示 |
| `git-shadow rebase [file]` | ベースラインを更新し shadow 変更を再適用 (3-way merge) |
| `git-shadow restore [file]` | 中断されたコミットやクラッシュからの復旧 |
| `git-shadow doctor` | hooks・設定の整合性・残留状態を診断 |

## 仕組み

1. **pre-commit hook**: shadow 変更を退避し、ベースラインを復元してインデックスを更新
2. **git commit**: クリーンなベースライン（shadow 変更なし）を記録
3. **post-commit hook**: 退避していた shadow 変更をワーキングツリーに復元

すべてのデータは `.git/shadow/` に保存されます。`.git/` 内にあるため自動的にコミット対象外です。

## 安全性

- **原子的書き込み**: 一時ファイル → rename パターンでデータ破損を防止
- **ロックファイル**: PID ベースのロックで並行操作を防止
- **ロールバック**: pre-commit の失敗時は自動的にロールバック
- **リカバリ**: `git-shadow restore` であらゆる中断状態から復旧可能

## ドキュメント

- [詳細な使い方ガイド](docs/usage.ja.md)
- [要件定義](docs/requirements.md)

## 動作要件

- Git 2.20+
- Rust 1.70+（ソースからビルドする場合）

## ライセンス

MIT
