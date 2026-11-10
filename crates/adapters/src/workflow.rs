use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use zcomposer_core::{IntegrationCandidate, State, Store};

use crate::{Result, executable, review::git_output, review_commit, run_turn};

pub fn implement_workflow(
    store: &mut Store,
    root: &Path,
    task: &str,
    tests: &[String],
) -> Result<i64> {
    let (test_program, test_args) = tests.split_first().ok_or("Informe o comando de testes.")?;
    let program = Path::new(test_program);
    let test_program = if program.is_absolute() {
        program.to_path_buf()
    } else if program.components().count() == 1 {
        executable(test_program)?
    } else {
        return Err("Use o nome de um executável no PATH ou seu caminho absoluto.".into());
    };
    let branch = project_branch(root)?;
    let base = head(root)?;
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let runs = root.join(".zcomposer/workflows");
    fs::create_dir_all(&runs)?;
    let run = runs.join(format!("{}-{nonce}", std::process::id()));
    fs::create_dir(&run)?;
    let worktree = run.join("worktree");
    let path = worktree.to_str().ok_or("Caminho do worktree inválido.")?;
    git_output(root, &["worktree", "add", "--detach", path, &base], 4096)?;
    eprintln!("Área de trabalho: {}", worktree.display());

    let prompt = format!(
        "Prepare um patch Git unificado para a tarefa abaixo, relativo à raiz do projeto. \
         Use caminhos a/ e b/, inclua o contexto das linhas e termine o patch com uma quebra de linha. \
         O Zcomposer vai aplicar o patch, criar o commit e executar o comando de testes.\n\n{task}"
    );
    let (implementation, state) = run_turn(
        store,
        &worktree,
        "codex-implement",
        task,
        &prompt,
        None,
        |_, _| {},
    )?;
    if state != State::Completed || head(&worktree)? != base {
        return Err(
            format!("Sessão {implementation}: implementação incompleta ou HEAD alterado.").into(),
        );
    }
    let answer: serde_json::Value = serde_json::from_str(&store.answer(implementation)?)?;
    let patch = answer["patch"]
        .as_str()
        .ok_or("O Codex não retornou um patch.")?;
    if patch.trim().is_empty() || patch.len() > 65_536 {
        return Err("O patch precisa conter alterações e ter até 64 KiB.".into());
    }
    apply_patch(&worktree, patch)?;
    ensure_local_data_untracked(&worktree)?;
    if git_output(&worktree, &["diff", "--cached", "--name-only"], 65_536)?.is_empty() {
        return Err("A implementação não produziu alterações para revisar.".into());
    }
    git_output(
        &worktree,
        &["commit", "-m", "Implement Zcomposer workflow task"],
        65_536,
    )?;
    let commit = head(&worktree)?;
    let log_path = run.join("tests.log");
    let log = fs::File::create_new(&log_path)?;
    eprintln!("Log dos testes: {}", log_path.display());
    let outcome = Command::new(test_program)
        .args(test_args)
        .current_dir(&worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .status()?;
    if !outcome.success() {
        return Err(format!(
            "Os testes falharam ({outcome}). Veja {}.",
            log_path.display()
        )
        .into());
    }
    if head(&worktree)? != commit || !clean(&worktree, false)? {
        return Err("O comando de testes alterou o commit ou deixou arquivos pendentes.".into());
    }

    let review = review_commit(store, &worktree, &commit)?;
    let candidate = IntegrationCandidate {
        branch,
        base,
        commit,
    };
    if !store.record_workflow_candidate(review, &candidate)? {
        return Err("O parecer não corresponde ao commit testado.".into());
    }
    Ok(review)
}

fn apply_patch(worktree: &Path, patch: &str) -> Result<()> {
    let mut child = Command::new(executable("git")?)
        .args([
            "--no-pager",
            "--no-replace-objects",
            "apply",
            "--index",
            "--",
        ])
        .current_dir(worktree)
        .env("GIT_NO_LAZY_FETCH", "1")
        .stdin(Stdio::piped())
        .spawn()?;
    let written = child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(patch.as_bytes());
    let status = child.wait()?;
    if !status.success() {
        return Err("O Git rejeitou o patch. Confira a sessão de implementação.".into());
    }
    written?;
    Ok(())
}

pub fn integrate_workflow(store: &Store, root: &Path, review: i64) -> Result<String> {
    let candidate = store
        .approved_candidate(review)?
        .ok_or("A integração exige uma revisão aprovada de um workflow com testes concluídos.")?;
    if project_branch(root)? != candidate.branch || !clean(root, true)? {
        return Err("Abra a branch de origem com a área de trabalho limpa para integrar.".into());
    }
    let current = head(root)?;
    if current == candidate.commit {
        return Ok(current);
    }
    if current != candidate.base {
        return Err("A branch de origem mudou. Rode um novo workflow sobre a base atual.".into());
    }
    git_output(
        root,
        &[
            "merge",
            "--ff-only",
            "--no-autostash",
            "--no-overwrite-ignore",
            &candidate.commit,
        ],
        65_536,
    )?;
    Ok(candidate.commit)
}

fn project_branch(root: &Path) -> Result<String> {
    if !git_output(root, &["rev-parse", "--show-prefix"], 4096)?
        .trim()
        .is_empty()
    {
        return Err("Execute o workflow na raiz do repositório Git.".into());
    }
    ensure_local_data_untracked(root)?;
    let branch = git_output(root, &["symbolic-ref", "--quiet", "HEAD"], 4096)?;
    Ok(branch.trim_end().to_owned())
}

fn head(root: &Path) -> Result<String> {
    Ok(
        git_output(root, &["rev-parse", "--verify", "HEAD^{commit}"], 256)?
            .trim()
            .to_owned(),
    )
}

fn ensure_local_data_untracked(root: &Path) -> Result<()> {
    if !git_output(root, &["ls-files", "--", ".zcomposer"], 4096)?.is_empty() {
        return Err("A pasta .zcomposer precisa ficar fora dos arquivos versionados.".into());
    }
    Ok(())
}

fn clean(root: &Path, exclude_local_data: bool) -> Result<bool> {
    let mut args = vec![
        "status",
        "--porcelain=v1",
        "--untracked-files=all",
        "--",
        ".",
    ];
    if exclude_local_data {
        args.push(":(exclude,top).zcomposer");
    }
    Ok(git_output(root, &args, 65_536)?.is_empty())
}
