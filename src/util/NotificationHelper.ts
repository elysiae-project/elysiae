import {
	requestPermission,
	sendNotification,
} from "@tauri-apps/plugin-notification";
import { getOption, setOption } from "./Settings";
import { resolveResource } from "@tauri-apps/api/path";

const areNotificationsPermitted = async (): Promise<boolean> => {
	if (await getOption("blockNotifications")) return false;
	const permission = await requestPermission();
	if (permission !== "granted") {
		await setOption("blockNotifications", true);
		return false;
	}
	return true;
};

export const broadcastNotification = async (message: string) => {
	if (await areNotificationsPermitted()) {
		console.log("Notifications Allowed")
		sendNotification({
			title: "Elysiae",
			body: message,
			icon: await resolveResource("appIcon.png"),
		});
	}
};
