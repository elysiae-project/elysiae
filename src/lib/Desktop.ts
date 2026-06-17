import { resolveResource } from "@tauri-apps/api/path";
import { BaseDirectory, writeTextFile } from "@tauri-apps/plugin-fs";
import {
	requestPermission,
	sendNotification,
} from "@tauri-apps/plugin-notification";
import type { Variants } from "../types";
import { getOption, setOption } from "./Settings";
import { variantToGameCode, variantToGameName } from "./VariantConverter";

/**
 * @returns Status of notification permission from DE + settings.
 */
const areNotificationsPermitted = async (): Promise<boolean> => {
	if (await getOption("blockNotifications")) return false;
	const permission = await requestPermission();
	if (permission !== "granted") {
		await setOption("blockNotifications", true);
		return false;
	}
	return true;
};

/**
 * Sends a notification to the active desktop environment
 * @param message The body of the notification
 */
export const broadcastNotification = async (message: string) => {
	if (await areNotificationsPermitted()) {
		sendNotification({
			title: "Elysiae",
			body: message,
			icon: await resolveResource("appIcon.png"),
		});
	}
};

/**
 * Creates a desktop shortcut for an installed game
 */
export const createDesktopShortcut = async (game: Variants) => {
	const gameName = variantToGameName[game];

	// TODO: Start caching game icons and other image assets to allow icons to be set for the dekstop shortcuts here
	const desktopEntry = `
		[Desktop Entry]
		Name=${gameName}
		Exec=xdg-open elysiae://open-game/${variantToGameCode[game]}
		Comment="Play with Elysiae"
		Icon=
		Type=Application
		Categories=Game
	`;

	await writeTextFile(`${gameName}.desktop`, desktopEntry, {
		baseDir: BaseDirectory.Desktop,
	});

	await writeTextFile(`applications/${gameName}.desktop`, desktopEntry, {
		baseDir: BaseDirectory.Data,
	});
};

/*
1.1 features

export const createSteamTitle = async() => {

}
*/
