# git-shadow 要件定義 v4

## 概要

Git リポジトリ内のファイルに対して「ローカル限定の変更」を管理する CLI ツール。
開発中は変更が反映された状態でファイルを使えるが、コミット時には自動的にローカル変更が剥がされ、Git の履歴には一切残らない。

主な用途は Claude Code の `CLAUDE.md` 運用であり、個人開発者が自分だけの指示やメモをコミット履歴を汚さずに管理することを目的とする。

## 実装言語

Rust（シングルバイナリ配布）

## ユースケース

### UC1: 既存の `CLAUDE.md` への個人的な追記

チームで共有している `CLAUDE.md` に、自分だけの指示（デバッグ用プロンプト、個人的なコーディング規約など）を追記したい。チームのコミット履歴は汚したくない。

### UC2: サブディレクトリへの `CLAUDE.md` 新規配置

`src/components/CLAUDE.md` など、まだリポジトリに存在しないファイルを新規作成してローカルでだけ使いたい。このファイル自体はコミットしない。

## 対象ファイルの分類

| 種別 | 説明 | 例 |
|---|---|---|
| **overlay** | 既存のトラッキング済みファイルにローカル変更を重ねる | ルートの `CLAUDE.md` に追記 |
| **phantom** | リポジトリに存在しないファイルをローカルだけで作成する | `src/components/CLAUDE.md` を新規作成 |

## 対象ファイルの制約

- **テキストファイル限定**: rebase コマンドが diff + 3-way merge を前提とするため、バイナリファイルは対象外とする。`git-shadow add` 時にバイナリ判定を行い、バイナリの場合は警告を出して拒否する。
- **サイズ上限**: 実用上の上限として 1MB を設定する。超過する場合は警告を出す（`--force` で突破可能）。
- **改行コード**: 内部処理はファイルをそのまま扱い、改行コードの変換は行わない。Git の `core.autocrlf` 設定はユーザーの責任とする。

## パスの正規化ルール

- config.json のキーおよび内部処理では、常に**リポジトリルートからの相対パス**を使用する。
- パス区切りは `/` に統一する（Windows 環境でも）。
- 先頭の `./` は除去する（`./CLAUDE.md` → `CLAUDE.md`）。
- `baselines/` および `stash/` 配下のファイル名は、パス文字列を **URL エンコード**してフラットに保存する。エンコード手順は以下の通り:
  1. `%` → `%25` に置換する（先にエスケープ文字自体を処理する）
  2. `/` → `%2F` に置換する
  3. デコード時は逆順（`%2F` → `/`、`%25` → `%`）で復元する
- この方式により、元のパスに `%` が含まれていても一意にデコードできる。
- 例: `src/components/CLAUDE.md` → `src%2Fcomponents%2FCLAUDE.md`
- 例: `docs/100%done.md` → `docs%2F100%25done.md`

## コマンド体系

### `git-shadow install`

Git hooks（pre-commit, post-commit, post-merge）をセットアップする。

```bash
git-shadow install
```

#### 既存 hook との共存

既存の hook ファイルが存在する場合は上書きせず、チェーン実行する構成を生成する。

- 既存の hook ファイルを `<hook-name>.pre-shadow` にリネームして退避する。
- 新しい hook ファイルは薄いラッパーとし、`git-shadow hook <hook-name>` を呼び出した後に退避した既存 hook を実行する。

生成される hook ファイルの例（pre-commit）:

```sh
#!/bin/sh
# git-shadow managed hook
git-shadow hook pre-commit
SHADOW_EXIT=$?
if [ $SHADOW_EXIT -ne 0 ]; then
  exit $SHADOW_EXIT
fi

# 既存 hook のチェーン実行
if [ -x .git/hooks/pre-commit.pre-shadow ]; then
  .git/hooks/pre-commit.pre-shadow "$@"
fi
```

既存 hook が失敗（非ゼロ exit）した場合、commit は中断され post-commit が走らない。この場合も stash/lock が残るため、`git-shadow restore` で復旧できる。

### `git-shadow hook <hook-name>`

hook から呼び出される内部サブコマンド。pre-commit / post-commit / post-merge の各処理を実行する。hook ファイルから呼ばれることを前提とし、ユーザーが直接実行することは想定しない。

```bash
git-shadow hook pre-commit
git-shadow hook post-commit
git-shadow hook post-merge
```

### `git-shadow doctor`

hooks と設定の状態を総合的に診断する。

```bash
git-shadow doctor
```

診断項目:

- hook ファイルが存在するか
- hook ファイルに実行権限があるか
- hook ファイルの内容が `git-shadow hook` を呼び出しているか
- 他の hook マネージャー（Husky, pre-commit, lefthook 等）との競合がないか
- config.json の整合性（管理対象ファイルが存在するか等）
- stash に残留ファイルがないか（残留している場合は「前回の commit が途中で中断された可能性があります。`git-shadow restore` を実行してください」と案内する）
- lockfile が残っていないか（残っている場合は PID を確認し、プロセスが存在しなければ stale lock として `git-shadow restore` を案内する）

### `git-shadow add <file>`

既存のトラッキング済みファイルを overlay 管理に登録する。

```bash
git-shadow add CLAUDE.md
```

- 現在の HEAD の内容をベースラインとして `.git/shadow/baselines/` に保存する
- `config.json` にエントリを追加する
- hooks 未インストール状態で実行した場合は警告を出す
- バイナリファイルの場合は拒否する
- サイズ上限を超える場合は警告を出す（`--force` で突破可能）

### `git-shadow add --phantom <file>`

リポジトリに存在しないファイルを phantom 管理に登録する。

```bash
git-shadow add --phantom src/components/CLAUDE.md
```

- デフォルトでは `.git/info/exclude` にエントリを自動追加する（冪等管理セクション方式、後述）
- `--no-exclude` フラグを指定すると `.git/info/exclude` への追加をスキップし、pre-commit hook による防御のみで運用する
- `config.json` にエントリを追加する
- hooks 未インストール状態で実行した場合は警告を出す

### `git-shadow remove <file>`

shadow 管理を解除する。

```bash
git-shadow remove CLAUDE.md
```

- overlay: ファイルの内容をベースライン（HEAD の内容）に戻す。**shadow 変更は破棄される。**
- phantom: ファイル自体は残すが管理対象から外す。`.git/info/exclude` の管理セクションから該当エントリを削除する。
- 実行前に確認プロンプトを表示する:
  - overlay: `"CLAUDE.md の shadow 変更が破棄されます。続行しますか？ [y/N]"`
  - phantom: `"src/components/CLAUDE.md を shadow 管理から解除します。ファイル自体は残ります。続行しますか？ [y/N]"`
- `--force` フラグで確認プロンプトをスキップできる（スクリプトからの利用向け）。
- TTY が接続されていない（非対話環境の）場合は `--force` 必須とし、未指定ならエラーで終了する。

### `git-shadow status`

管理対象ファイルの一覧と状態を表示する。

```bash
git-shadow status
```

出力例（正常時）:

```
管理対象ファイル:

  CLAUDE.md (overlay)
    ベースライン: abc1234
    shadow変更: +12 行 / -0 行

  src/components/CLAUDE.md (phantom)
    exclude: .git/info/exclude
    ファイルサイズ: 1.2 KB
```

出力例（不整合検出時）:

```
  CLAUDE.md (overlay)
    ⚠ ワーキングツリーの内容が shadow の期待状態と一致しません
    → git stash や git checkout で変更が失われた可能性があります
    → git-shadow restore CLAUDE.md で復元できます

  CLAUDE.md (overlay)
    ⚠ ベースラインが古くなっています (abc1234 → def5678)
    → git-shadow rebase CLAUDE.md を実行してください
```

出力例（commit 中断後）:

```
  ⚠ stash に残留ファイルがあります（前回の commit が途中で中断された可能性があります）
    → git-shadow restore を実行してください

  ⚠ lockfile が残っています（PID 12345 は既に終了しています）
    → git-shadow restore を実行してください
```

#### 不整合の検出対象と重症度

不整合は重症度に応じて **Hard fail（コミット中断）** と **Soft warn（警告のみ）** に分類する。

**Hard fail（pre-commit でコミットを中断する）:**

| 状況 | 検出方法 |
|---|---|
| stash に残留ファイルがある | `.git/shadow/stash/` にファイルが存在する |
| lockfile が残っている（stale） | `.git/shadow/lock` が存在し、記録された PID のプロセスが存在しない |
| shadow 管理対象ファイルが消失している | overlay 対象のファイルがワーキングツリーに存在しない |
| ベースラインファイルが壊れている・消失している | `.git/shadow/baselines/` の該当ファイルが存在しない or 読み取り不可 |

**Soft warn（警告を表示するがコミットは続行する）:**

| 状況 | 検出方法 |
|---|---|
| ベースラインのコミットがずれている | `baseline_commit` と HEAD の比較 |
| phantom の `--no-exclude` 運用で `git status` に表示されている | `exclude_mode` が `none` の phantom がある |

### `git-shadow diff [file]`

shadow 変更の差分を表示する。

```bash
git-shadow diff
git-shadow diff CLAUDE.md
```

- overlay: ベースラインと現在のファイル内容の diff を表示する
- phantom: ファイルの全内容を表示する（ベースラインが存在しないため）

### `git-shadow rebase [file]`

ベースラインを現在の HEAD の内容に更新し、shadow 変更を再適用する。

```bash
git-shadow rebase CLAUDE.md
```

処理フロー:

1. 現在のファイル内容（ベースライン + shadow 変更）を取得する
2. 旧ベースラインと現在のファイル内容の diff を算出する（= shadow 変更分）
3. 新しい HEAD の内容を新ベースラインとして保存する
4. 新ベースラインに shadow 変更分を 3-way merge で適用する
5. コンフリクトが発生した場合はコンフリクトマーカー付きで出力し、手動解決を促す
6. `config.json` の `baseline_commit` を更新する

### `git-shadow restore [file]`

異常状態からの完全リカバリを行うコマンド。以下の処理をすべて実行する:

```bash
git-shadow restore
git-shadow restore CLAUDE.md
```

処理フロー:

1. stash にファイルが存在すれば、ワーキングツリーに復元する
2. stash をクリーンアップする
3. lockfile が存在すれば削除する
4. 復旧結果のサマリーを表示する

このコマンドは以下のどのケースでも「`restore` を実行すれば通常状態に戻る」ことを保証する:

- pre-commit の途中でエラーが発生した場合
- pre-commit は成功したが commit 自体が不成立だった場合（commit-msg での中断、エディタを閉じた中断等）
- 既存 hook のチェーン実行が失敗して commit が中断された場合
- `git stash` や `git checkout` で shadow 変更が失われた場合（stash に前回退避分が残っていれば復元できる）

## 内部データ構造

### 保存先

すべてのデータは `.git/shadow/` 配下に保存する。`.git/` 内にあるため自動的にコミット対象外となる。

```
.git/shadow/
├── config.json          # 管理対象ファイルのリスト・メタデータ
├── lock                 # 実行中フラグ（lockfile、PID とタイムスタンプを記録）
├── baselines/           # overlay 対象のベースライン（HEAD の内容のスナップショット）
│   └── <url_encoded_path>
└── stash/               # pre-commit 時の退避先
    └── <url_encoded_path>
```

### config.json

```json
{
  "version": 1,
  "files": {
    "CLAUDE.md": {
      "type": "overlay",
      "baseline_commit": "abc1234def5678",
      "exclude_mode": "none",
      "added_at": "2026-02-07T12:00:00Z"
    },
    "src/components/CLAUDE.md": {
      "type": "phantom",
      "exclude_mode": "git_info_exclude",
      "added_at": "2026-02-07T12:00:00Z"
    }
  }
}
```

| フィールド | 説明 |
|---|---|
| `version` | config フォーマットのバージョン |
| `type` | `overlay` または `phantom` |
| `baseline_commit` | ベースラインを取得した時点のコミットハッシュ（overlay のみ） |
| `exclude_mode` | `git_info_exclude`（デフォルト）または `none`（`--no-exclude` 指定時）。overlay では常に `none` |
| `added_at` | 管理対象に追加した日時 |

## Git Hooks の動作

### pre-commit

コミット直前に実行される。shadow 変更をファイルから剥がし、コミットに含まれないようにする。

```
0. lockfile の取得
   .git/shadow/lock を作成する（PID とタイムスタンプを記録）。
   既に存在する場合:
     - 記録された PID のプロセスがまだ生きていれば処理を中断する。
     - プロセスが存在しなければ stale lock として扱い、
       「git-shadow restore を実行してください」と表示して処理を中断する。
       → Hard fail

1. 不整合チェック
   管理対象ファイルの状態を検証する。
   Hard fail 対象の不整合が見つかった場合はコミットを中断する。
   Soft warn 対象の不整合は警告を表示し、コミットは続行する。

2. 部分ステージの検出
   overlay 管理対象ファイルについて、以下の条件を検査する:
     (a) index ≠ HEAD（ステージされた変更がある）
     (b) worktree ≠ index（未ステージの変更がある）
   (a) と (b) が同時に成立する場合、部分ステージと判断してコミットを中断する。
   エラーメッセージ:
     「git-shadow 管理下のファイル <file> に部分ステージが検出されました。
      git add <file> でファイル全体をステージしてから再度コミットしてください。」

3. overlay ファイルごとに:
   a. 現在のファイル内容を .git/shadow/stash/ に退避する
      書き込みは一時ファイルに行い、完了後に rename で配置する（原子的書き込み）
   b. ベースライン（.git/shadow/baselines/）の内容でファイルを上書きする
   c. git add <file> でステージングを更新する

4. phantom ファイルごとに:
   a. 現在のファイル内容を .git/shadow/stash/ に退避する
      （原子的書き込み）
   b. ステージングから除外する。以下の順で試行する:
      i.   git rm --cached --ignore-unmatch <file>
      ii.  失敗した場合: git restore --staged <file>
      iii. 失敗した場合（HEAD が unborn 等）: git reset -- <file>
      iv.  すべて失敗した場合: コミットを中断する
           「phantom ファイル <file> をステージングから除外できませんでした。
            手動で git reset -- <file> を実行してください。」
```

### post-commit

コミット直後に実行される。退避したファイルを復元する。

```
管理対象ファイルごとに:
  1. .git/shadow/stash/ から元の内容をワーキングツリーに復元する
     - 復元はベストエフォートで全件試行する（1件の失敗で即中断しない）
     - 復元に成功したファイルの stash エントリのみ削除する
     - 復元に失敗したファイルの stash エントリは残す

復元結果の判定:
  - 全件成功: stash クリーンアップ完了、lockfile を削除する
  - 一部失敗:
    - lockfile は削除しない（未復元ファイルがあることを検出可能にするため）
    - 失敗したファイルの一覧を表示し、git-shadow restore の実行を案内する
```

### post-merge

`git pull` や `git merge` の直後に実行される。ベースラインのずれを検出する。

```
overlay ファイルごとに:
  1. config.json の baseline_commit と現在の HEAD を比較する
  2. 対象ファイルの HEAD の内容がベースラインと異なっていれば警告を表示する:
     "⚠ CLAUDE.md のベースラインが古くなっています。
      git-shadow rebase CLAUDE.md を実行してください"
```

自動 rebase は行わない。ユーザーが明示的に `git-shadow rebase` を実行する。

### pre-commit 失敗時のロールバック

pre-commit の処理中にエラーが発生した場合（ファイル書き込み失敗、Git コマンド失敗等）:

1. 既に stash に退避済みのファイルをワーキングツリーに復元する
2. ステージングを元の状態に戻す
3. lockfile を削除する
4. 非ゼロの exit code で終了し、コミットを中断する

### commit 不成立時（post-commit 不発）の復旧

pre-commit は正常に完了したが、その後 commit 自体が成立しなかった場合（commit-msg hook の失敗、エディタを閉じての中断、既存 hook のチェーン実行失敗等）:

- post-commit が実行されないため、ワーキングツリーはベースライン状態のまま（shadow 変更が剥がれたまま）になる。
- stash と lockfile が残った状態になる。
- 次回の `git-shadow` コマンド実行時（status, doctor, 次の commit 等）で stale lock を検出し、`git-shadow restore` の実行を案内する。
- `git-shadow restore` が stash 復元 + stash クリーンアップ + lock 削除を一括で行い、通常状態に復帰する。

## phantom ファイルの除外管理

phantom ファイルが `git status` に未追跡ファイルとして表示されること、および誤って `git add` でコミットされることを防ぐ。

### デフォルト動作（`.git/info/exclude` 方式）

`git-shadow add --phantom <file>` 実行時に `.git/info/exclude` にエントリを自動追加する。`.git/info/exclude` は `.git/` 内にあるためコミット対象にならない。

#### 冪等管理セクション方式

`.git/info/exclude` 内に管理セクション（開始・終了マーカー）を設け、git-shadow が管理するエントリはすべてこのセクション内に配置する。

```
# 既存のユーザー設定
*.log
tmp/

# >>> git-shadow managed (DO NOT EDIT) >>>
src/components/CLAUDE.md
tests/fixtures/CLAUDE.md
# <<< git-shadow managed <<<
```

管理ルール:

- **追加時**: セクション内に同一パスが既に存在すればスキップする（重複防止）。セクションが存在しなければ新規作成する。
- **削除時（`git-shadow remove`）**: セクション内から該当パスのみを削除する。セクション外のエントリには一切触れない。
- **セクションが空になった場合**: セクションマーカーごと削除する。

### `--no-exclude` フラグ指定時（hook 防御のみ方式）

`.git/info/exclude` への追加をスキップする。`git status` には未追跡ファイルとして表示されるが、pre-commit hook でステージングから除外されるためコミットには含まれない。

## 原子性の保証

hook 処理の途中でクラッシュや異常終了が発生しても、データが破損しないようにする。

### lockfile

- `.git/shadow/lock` を hook 処理の開始時に作成し、終了時に削除する。
- lockfile には PID とタイムスタンプを記録する。
- lockfile が既に存在する場合は記録された PID を確認する:
  - プロセスが生存中: 二重起動と判断し、処理を中断する。
  - プロセスが存在しない: stale lock と判断し、`git-shadow restore` の実行を案内して処理を中断する。

### 原子的ファイル書き込み

stash やベースラインへのファイル書き込みは、同一ディレクトリ内の一時ファイルに書き込んでから `rename` で配置する。これにより、書き込み途中でクラッシュしても不完全なファイルが残らない。

### ロールバック

pre-commit の処理中にエラーが発生した場合、既に退避・上書きしたファイルを可能な限り元の状態に戻す。完全なロールバックが不可能な場合でも、stash にファイルが残っている状態を保証し、`git-shadow restore` で復旧できるようにする。

## `git commit --no-verify` への対応

`--no-verify` を指定すると pre-commit hook がスキップされるため、shadow 変更の剥がし処理が実行されず、shadow 変更がそのままコミットに含まれる。post-commit はスキップされない場合があるが、pre-commit での剥がし処理が行われていないため、post-commit での復元処理は実行しない（stash が存在しないため）。

これは Git の仕様上回避不可能であり、ユーザーの自己責任とする。また、pre-commit が実行されなかった場合は stash も lock も作成されないため、ツール側で「shadow 変更がコミットされた」ことを事後に検知することも困難である。ドキュメントに注意事項として記載する。

## スコープ外（将来検討）

以下は初期実装には含めない。

- チームでの共有機能（テンプレート、export/import 等）
- post-checkout hook によるブランチ切り替え時の自動対応（ただし status と pre-commit の不整合チェックでずれは検出できる）
- post-merge での自動 rebase（`--auto` フラグ）
- `git-shadow remove --keep`（shadow 変更を保持したまま管理解除し、通常のコミット対象にする）
- GUI / TUI
