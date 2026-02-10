# Feature Request: ディレクトリの phantom サポート

## 概要

`git-shadow add --phantom` でディレクトリを登録した場合、pre-commit フックで `std::fs::read()` がディレクトリに対して呼ばれ、`Is a directory (os error 21)` で失敗する。

## 再現手順

```bash
git-shadow add --phantom .claude/
git-shadow add --phantom codemaps/
git commit  # → Error: failed to read .claude/ Caused by: Is a directory (os error 21)
```

## ユースケース

Claude Code の `.claude/` ディレクトリや、コードマップ (`codemaps/`)、レポート (`.reports/`) など、**ローカル専用のディレクトリツリー全体**をgit管理対象外にしたい。

現在の回避策は `.git/info/exclude` に手書きすることだが、git-shadow で一元管理できると：
- `git-shadow status` で phantom ディレクトリも含めた全管理対象を確認できる
- `git-shadow remove` で exclude エントリも含めたクリーンアップができる
- 管理方法が統一される

## 問題の原因

### 1. `add --phantom` がディレクトリを検出しない

`src/commands/add.rs:80-109` の `add_phantom()` は `is_tracked()` チェックのみで、パスがファイルかディレクトリかを判定していない。そのため、登録自体は成功してしまう。

### 2. pre-commit フックがファイル前提で `read()` する

`src/hooks/pre_commit.rs:205-223` の `process_phantom()`:

```rust
if worktree_path.exists() {
    let content = std::fs::read(&worktree_path)  // ← ここでディレクトリに対してread
        .with_context(|| format!("failed to read {}", file_path))?;
    fs_util::atomic_write(&stash_path, &content)?;
    tx.stashed_phantoms.push(file_path.to_string());
}
```

`std::fs::read()` はディレクトリに対して `EISDIR` を返すため失敗する。

### 3. post-commit のリストアも同様にファイル前提

`src/hooks/post_commit.rs:39-43` で stash から `read` → worktree に `write` しているが、ディレクトリの場合これは意味をなさない。

### 4. status コマンドのサイズ表示

`src/commands/status.rs:112-114` で `std::fs::metadata()` → `metadata.len()` を表示しているが、ディレクトリの場合 `len()` はファイルシステム依存の無意味な値を返す。

## 提案する対応

### 方針: phantom ディレクトリは「exclude のみ管理」

phantom ディレクトリの本質は **`.git/info/exclude` への登録** と **`git rm --cached` による unstage** であり、内容の stash/restore は不要。ファイルの phantom と異なり、ディレクトリの中身をバイト列として stash する必要はない。

### 具体的な変更

#### A. config.rs: エントリにディレクトリフラグを追加

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    #[serde(rename = "type")]
    pub file_type: FileType,
    #[serde(default)]              // 追加: 後方互換
    pub is_directory: bool,         // 追加
    // ... 既存フィールド
}
```

`is_directory: false` のデフォルトで既存の config.json との後方互換を維持。

#### B. commands/add.rs: ディレクトリ検出

```rust
fn add_phantom(/* ... */) -> Result<()> {
    let full_path = git.root.join(normalized);
    let is_dir = full_path.is_dir();

    // ディレクトリの場合は末尾スラッシュ付きで exclude に登録
    let exclude_path = if is_dir && !normalized.ends_with('/') {
        format!("{}/", normalized)
    } else {
        normalized.to_string()
    };

    // ... exclude 登録処理 ...

    config.add_phantom(normalized.to_string(), exclude_mode, is_dir)?;
    Ok(())
}
```

#### C. hooks/pre_commit.rs: ディレクトリはスキップ

```rust
fn process_phantom(git: &GitRepo, file_path: &str, entry: &FileEntry, tx: &mut PreCommitTransaction) -> Result<()> {
    if entry.is_directory {
        // ディレクトリ: stash不要、unstageのみ
        git.unstage_phantom(file_path)?;
        return Ok(());
    }
    // 既存のファイル処理 ...
}
```

`process_files()` から `FileEntry` を渡すように変更が必要。

#### D. hooks/post_commit.rs: ディレクトリ stash はスキップ

stash にディレクトリエントリが存在しないため、既存ロジックで自然にスキップされる。追加変更は不要。

#### E. commands/status.rs: ディレクトリ表示の改善

```rust
FileType::Phantom => {
    let label = if entry.is_directory { "phantom dir" } else { "phantom" };
    println!("  {} ({})", file_path, label);
    // ...
    if worktree_path.exists() {
        if entry.is_directory {
            // ファイル数を表示
            let count = std::fs::read_dir(&worktree_path)?.count();
            println!("    contents: {} entries", count);
        } else {
            let metadata = std::fs::metadata(&worktree_path)?;
            println!("    file size: {}", format_size(metadata.len()));
        }
    }
}
```

### 影響範囲

| ファイル | 変更内容 |
|---------|---------|
| `src/config.rs` | `FileEntry` に `is_directory` フィールド追加 |
| `src/commands/add.rs` | `add_phantom()` でディレクトリ検出、exclude パスに `/` 付与 |
| `src/hooks/pre_commit.rs` | `process_phantom()` でディレクトリ時に stash スキップ |
| `src/commands/status.rs` | ディレクトリ表示の改善 |
| テスト | 各ファイルにディレクトリケースのテスト追加 |

`post_commit.rs`, `commands/remove.rs`, `commands/rebase.rs` は変更不要（ディレクトリの stash が存在しないため自然にスキップ）。

## 代替案

### 案B: ディレクトリを登録時に個別ファイルに展開

ディレクトリ配下の全ファイルを個別に phantom 登録する。ただし：
- ファイル追加時に再登録が必要
- config.json が肥大化する
- `.git/info/exclude` にディレクトリパターンを書く方がシンプル

→ **案Aの方が自然で簡潔**

### 案C: ディレクトリの phantom 登録を禁止してエラーにする

最小変更だが、ユースケース（ディレクトリ単位の除外管理）が満たせない。

→ **最低限のフォールバックとして、案Aが実装されるまでの暫定対応に適している**

---

## 対応結果

**ステータス: 実装完了**

案A「phantom ディレクトリは exclude のみ管理」で実装した。提案内容をほぼそのまま採用し、加えて提案で「変更不要」とされていた `remove.rs` と `diff.rs` にも対応を入れた。

### 実装内容

提案どおりの変更 (A-E) に加え、以下を追加で対応:

| ファイル | 変更内容 |
|---------|---------|
| `src/config.rs` | `is_directory` フィールド追加、`#[serde(default)]` + `#[serde(skip_serializing_if)]` で後方互換 |
| `src/path.rs` | `normalize_path()` で末尾 `/` をストリップ（config キーの一貫性確保） |
| `src/commands/add.rs` | ディレクトリ検出、exclude に末尾 `/` 付与、出力メッセージ分岐 |
| `src/hooks/pre_commit.rs` | `process_phantom()` に `&FileEntry` を渡し、ディレクトリ時に stash スキップ |
| `src/hooks/post_commit.rs` | 変更なし（提案どおり、stash が存在しないため自然にスキップ） |
| `src/commands/status.rs` | `(phantom dir)` ラベル、エントリ数表示、`is_dir()` チェック |
| `src/commands/diff.rs` | **追加対応**: ディレクトリ phantom でエントリ数を表示（`read_to_string()` の EISDIR 回避） |
| `src/commands/doctor.rs` | **追加対応**: ディレクトリ phantom は `is_dir()` で存在チェック |
| `src/commands/remove.rs` | **追加対応**: exclude エントリの末尾 `/` を一致させて正しく削除、確認プロンプトも分岐 |
| `docs/usage.md` | Phantom Directories セクション追加 |
| `docs/usage.ja.md` | Phantom ディレクトリ セクション追加 |

### テスト

18 テスト追加（146 → 164）:
- Unit: config (4), path (3), add (4), pre_commit (1), doctor (2), remove (2)
- E2E: ディレクトリ phantom フルサイクル (1), overlay + directory phantom 混合 (1)

### 提案との差分

提案では `remove.rs` は「変更不要」としていたが、実際には exclude エントリがファイルでは `path`、ディレクトリでは `path/` で登録されるため、削除時にも末尾 `/` の一致が必要だった。同様に `diff.rs` も `read_to_string()` で EISDIR が発生するため対応した。
