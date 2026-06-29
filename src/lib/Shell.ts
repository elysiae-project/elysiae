import { appDataDir } from "@tauri-apps/api/path";
import { Command } from "@tauri-apps/plugin-shell";

/**
 * Executes a command on the shell
 *
 * @param command Any command
 * @param env (optional) environment variables
 */
export const executeShellCommand = async (
	command: string,
	env?: Record<string, string> | undefined,
): Promise<void> => {
	await Command.create("sh", ["-c", command], {
		env: env,
	}).execute();
};

/**
 * Executes a command of a binary found in the Elysiae's app data directory
 *
 * @param binaryPath Path to binary, relative to the app data directory
 * @param args Arguments to pass into command
 * @param env (optional) environment variables
 */
export const executeLocalBinary = async (
	binaryPath: string,
	args?: string,
	env?: Record<string, string> | undefined,
): Promise<void> => {
	const appData = await appDataDir();
	await executeShellCommand(
		`${appData}/${binaryPath} ${typeof args !== "undefined" ? args : ""}`,
		env,
	).catch((e) => {
		throw new Error(e);
	});
};
