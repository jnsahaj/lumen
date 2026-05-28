/**
 * lumen extension for the Pi coding agent.
 *
 * Pops lumen up after every agent turn (and on `/lumen-review`). When the
 * user submits annotations, injects them as the next user message so the
 * agent reacts as if the user had typed them. The agent never invokes
 * lumen — Pi does.
 *
 * Disable the auto-trigger by setting LUMEN_AUTO_REVIEW=0 in the env.
 */

import { spawnSync } from "node:child_process";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

const LUMEN_BIN = process.env.LUMEN_BIN ?? "lumen";
const AUTO_REVIEW = process.env.LUMEN_AUTO_REVIEW !== "0";

function hasUncommittedChanges(cwd: string): boolean {
	const result = spawnSync("git", ["diff", "--quiet", "HEAD", "--"], {
		cwd,
		stdio: "ignore",
	});
	// `git diff --quiet` exits 1 when there are changes, 0 when clean.
	return result.status === 1;
}

type LumenRun = { status: number | null; output: string; error: string | null };

function runLumenReview(cwd: string, args: string[]): LumenRun {
	const result = spawnSync(LUMEN_BIN, ["diff", ...args], {
		cwd,
		// stdin: inherit so keystrokes flow; stdout: pipe so we capture
		// annotations; stderr: inherit so any lumen errors are visible.
		// lumen auto-routes its TUI to /dev/tty when stdout is captured.
		stdio: ["inherit", "pipe", "inherit"],
		env: process.env,
		encoding: "utf8",
	});

	if (result.error) {
		return { status: null, output: "", error: result.error.message };
	}
	return { status: result.status, output: (result.stdout ?? "").trim(), error: null };
}

async function reviewAndInject(
	pi: ExtensionAPI,
	ctx: ExtensionContext,
	args: string[],
	{ silentOnClean }: { silentOnClean: boolean },
): Promise<void> {
	if (!ctx.hasUI) return;

	if (!hasUncommittedChanges(ctx.cwd)) {
		if (!silentOnClean) {
			ctx.ui.notify("lumen: no uncommitted changes to review", "info");
		}
		return;
	}

	// Suspend Pi's TUI, hand the terminal to lumen, restart Pi's TUI.
	// The `ctx.ui.custom` pattern is the same one Pi's own interactive-shell
	// example uses for vim/htop/etc.
	const result = await ctx.ui.custom<LumenRun>((tui, _theme, _kb, done) => {
		tui.stop();
		process.stdout.write("\x1b[2J\x1b[H");

		const run = runLumenReview(ctx.cwd, args);

		tui.start();
		tui.requestRender(true);
		done(run);

		return { render: () => [], invalidate: () => {} };
	});

	if (result.error) {
		ctx.ui.notify(`lumen: ${result.error}`, "error");
		return;
	}

	if (result.status !== 0) {
		ctx.ui.notify(`lumen exited with status ${result.status}`, "warning");
		return;
	}

	if (!result.output) {
		// User pressed `q` instead of `s` — they looked but didn't send feedback.
		return;
	}

	pi.sendUserMessage(result.output);
}

export default function (pi: ExtensionAPI) {
	if (AUTO_REVIEW) {
		pi.on("agent_end", async (_event, ctx) => {
			try {
				await reviewAndInject(pi, ctx, [], { silentOnClean: true });
			} catch (err) {
				ctx.ui.notify(
					`lumen review failed: ${err instanceof Error ? err.message : String(err)}`,
					"error",
				);
			}
		});
	}

	pi.registerCommand("lumen-review", {
		description: "Open lumen on the diff; annotations get sent back to the agent",
		handler: async (args, ctx) => {
			const argv = (args ?? "").trim().length > 0 ? (args as string).trim().split(/\s+/) : [];
			try {
				await reviewAndInject(pi, ctx, argv, { silentOnClean: false });
			} catch (err) {
				ctx.ui.notify(
					`lumen review failed: ${err instanceof Error ? err.message : String(err)}`,
					"error",
				);
			}
		},
	});
}
