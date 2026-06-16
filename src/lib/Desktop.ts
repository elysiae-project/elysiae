import {
	desktopDir,
	join,
	resolveResource,
} from "@tauri-apps/api/path";
import { writeTextFile } from "@tauri-apps/plugin-fs";
import {
	requestPermission,
	sendNotification,
} from "@tauri-apps/plugin-notification";
import type { Variants } from "../types";
import { getOption, setOption } from "./Settings";
import {
	variantToGameCode,
	variantToGameName,
} from "./VariantConverter";

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

export const createDesktopShortcut = async (game: Variants) => {
	const gameName = variantToGameName[game];
	const filePath = await join(await desktopDir(), `${gameName}.desktop`);

	const desktopEntry = `
		[Desktop Entry]
		Name=${gameName}
		Exec=elysiae://open-game/${variantToGameCode[game]}
		Comment="Play with Elysiae"
		Icon=
		Type=Application
		Categories=Game
		
	`;

	await writeTextFile(filePath, desktopEntry);
};

/*
1.1 features

export const createSteamTitle = async() => {

}
*/
