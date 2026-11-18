/**
 * `/lumiere [args]` opens a Lumiere review, and when the review is sent with `s`
 * hands the annotations to the agent as a user message.
 *
 * Inside Rex the review opens in a split pane beside pi. Elsewhere pi's UI is
 * suspended while Lumiere owns the terminal.
 */

import { spawnSync } from "node:child_process";
import { closeSync, existsSync, mkdtempSync, openSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI, ExtensionCommandContext } from "@earendil-works/pi-coding-agent";

const LUMIERE_BIN = process.env.LUMIERE_BIN ?? "lumiere";
const DEFAULT_REVIEW_ARGS = ["diff", "origin/main...-"];
const REX_SPLIT_DIRECTION = "right";
const REX_SPLIT_RATIO = "70";

/** Runs the command given as arguments with stdout redirected to the file named by `$0` and stderr to `$1`. */
const REDIRECT_OUTPUT_TO_FIRST_ARGS = 'out="$0"; err="$1"; shift; "$@" > "$out" 2> "$err"';

const ANSI_STYLE = /\x1b\[[0-9;]*m/g;

const REVIEW_PROMPT = `I reviewed the changes in Lumiere and left the comments below.

Each comment names its file, its line or line range, and the side it is on: LEFT is the old version, RIGHT is the new one. When several repositories were reviewed, comments are grouped under a "# Repository: name (path)" heading.

Read the comments and inspect the relevant code. Respond here to each comment, grouped by repository and file, explaining whether you agree and what changes would address it.

Discuss the comments before implementing. Only make changes after explicit user approval.`;

type ReviewRun = { status: number | null; error?: string };

type RexPane = { session: string; block: string };

async function isOnPath(pi: ExtensionAPI, command: string): Promise<boolean> {
	const result = await pi.exec("sh", ["-c", 'command -v "$0"', command]);
	return result.code === 0;
}

async function rexPane(pi: ExtensionAPI): Promise<RexPane | undefined> {
	const { REX_SESSION: session, REX_BLOCK: block } = process.env;
	if (!session || !block || !(await isOnPath(pi, "rex"))) return undefined;
	return { session, block };
}

async function runInRexSplit(
	pi: ExtensionAPI,
	pane: RexPane,
	cwd: string,
	reviewArgs: string[],
	annotationsPath: string,
	errorsPath: string,
): Promise<ReviewRun> {
	const result = await pi.exec("rex", [
		"split",
		"-s",
		pane.session,
		"-b",
		pane.block,
		`--split=${REX_SPLIT_DIRECTION}`,
		"--ratio",
		REX_SPLIT_RATIO,
		"--wait",
		"-c",
		cwd,
		"--",
		"sh",
		"-c",
		REDIRECT_OUTPUT_TO_FIRST_ARGS,
		annotationsPath,
		errorsPath,
		LUMIERE_BIN,
		...reviewArgs,
	]);
	return { status: result.code, error: result.stderr.trim() };
}

async function runInPlace(
	ctx: ExtensionCommandContext,
	reviewArgs: string[],
	annotationsPath: string,
	errorsPath: string,
): Promise<ReviewRun> {
	const run = await ctx.ui.custom<ReviewRun>((tui, _theme, _keybindings, done) => {
		tui.stop();
		process.stdout.write("\x1b[2J\x1b[H");

		// Lumiere draws on /dev/tty when its stdout is not a terminal.
		const annotationsFile = openSync(annotationsPath, "w");
		const errorsFile = openSync(errorsPath, "w");
		const result = spawnSync(LUMIERE_BIN, reviewArgs, {
			cwd: ctx.cwd,
			stdio: ["inherit", annotationsFile, errorsFile],
		});
		closeSync(annotationsFile);
		closeSync(errorsFile);

		tui.start();
		tui.requestRender(true);
		done(result.error ? { status: null, error: result.error.message } : { status: result.status });

		return { render: () => [], invalidate: () => {} };
	});
	return run ?? { status: null, error: "pi could not hand over the terminal" };
}

async function review(pi: ExtensionAPI, ctx: ExtensionCommandContext, args: string): Promise<void> {
	if (ctx.mode !== "tui") {
		ctx.ui.notify("/lumiere needs pi's interactive terminal UI", "warning");
		return;
	}
	if (!(await isOnPath(pi, LUMIERE_BIN))) {
		ctx.ui.notify(`${LUMIERE_BIN} was not found. Install it or set LUMIERE_BIN.`, "error");
		return;
	}

	const reviewArgs = args.trim() ? args.trim().split(/\s+/) : DEFAULT_REVIEW_ARGS;
	const tempDir = mkdtempSync(join(tmpdir(), "lumiere-"));
	const annotationsPath = join(tempDir, "annotations.md");
	const errorsPath = join(tempDir, "errors.txt");
	try {
		const pane = await rexPane(pi);
		const run = pane
			? await runInRexSplit(pi, pane, ctx.cwd, reviewArgs, annotationsPath, errorsPath)
			: await runInPlace(ctx, reviewArgs, annotationsPath, errorsPath);

		if (run.status !== 0) {
			const lumiereErrors = existsSync(errorsPath)
				? readFileSync(errorsPath, "utf8").replace(ANSI_STYLE, "").trim()
				: "";
			const reason = lumiereErrors || run.error;
			const detail = reason ? `: ${reason}` : "";
			ctx.ui.notify(`lumiere did not finish (status ${run.status})${detail}`, "error");
			return;
		}

		const annotations = readFileSync(annotationsPath, "utf8").trim();
		if (!annotations) {
			ctx.ui.notify("lumiere: no annotations were sent", "info");
			return;
		}

		const message = `${REVIEW_PROMPT}\n\n${annotations}`;
		if (ctx.isIdle()) {
			pi.sendUserMessage(message);
		} else {
			pi.sendUserMessage(message, { deliverAs: "followUp" });
		}
	} finally {
		rmSync(tempDir, { recursive: true, force: true });
	}
}

export default function (pi: ExtensionAPI) {
	pi.registerCommand("lumiere", {
		description: "Review changes in Lumiere and send the annotations to the agent",
		handler: (args, ctx) => review(pi, ctx, args),
	});
}
