import { remove } from "../lib/Fs";
import { Variants } from "../types";
import { getActiveGameCode } from "./AppFunctions";
import { setOption } from "./Settings";

export const settingsDetails = [
	{
		name: "Preferred Voice-Over Language",
		boundTo: "voLanguage",
		description:
			"\x48\x49\x33 only supports Chinese/Japanese depending on region",
		type: "dropdown",
		values: ["English", "Japanese", "Chinese", "Korean"],
		onChange: async (newVoLang: string) => {
			await setOption("voLanguage", newVoLang);
		},
	},
	{
		name: "Allow Notifications",
		boundTo: "blockNotifications",
		type: "dropdown",
		values: ["Allow", "Disabled"],
		onChange: async () => {},
	},
	{
		name: "Verify Game Files",
		type: "button",
		onClick: async (game: Variants) => {},
	},
	{
		name: "Uninstall Game",
		type: "button",
		onClick: async (game: Variants) => {
			await remove(getActiveGameCode(game));
		},
	},
];
