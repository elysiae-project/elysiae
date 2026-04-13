import {
	isPermissionGranted,
	requestPermission,
	sendNotification,
} from "@tauri-apps/plugin-notification";
import { getOption, setOption } from "./Settings";

export const allowNotifications = async (): Promise<boolean> => {
	return new Promise((resolve, reject) => {
		getOption<boolean>("requestNotifications")
			.then((allow) => {
				if (!allow) return false;
				isPermissionGranted()
					.then((alreadyGranted) => {
						if (alreadyGranted) resolve(true);

						requestPermission()
							.then((res) => {
								if (res === "granted") {
									resolve(true);
								} else {
									setOption("requestNotifications", false).then(() =>
										resolve(false),
									);
								}
							})
							.catch(reject);
					})
					.catch(reject);
			})
			.catch(reject);
	});
};

export const broadcastNotification = async (message: string) => {
	if (await allowNotifications()) {
		sendNotification({
			title: "Elysiae",
			body: message,
		});
	}
};
