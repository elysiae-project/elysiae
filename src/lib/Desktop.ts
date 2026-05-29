import { resolveResource } from "@tauri-apps/api/path";
import {
	requestPermission,
	sendNotification,
} from "@tauri-apps/plugin-notification";
import { getOption, setOption } from "./Settings";

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

/*
1.1 features

export const createDesktopShortcut = async (game: Variants) => {

};


export const createSteamTitle = async() => {

}
*/
