import { remove } from "../lib/Fs";
import { Variants } from "../types";
import { getActiveGameCode } from "./AppFunctions";
import { setOption } from "./Settings";

export const settingsDetails = [
	{
		name: "Voice Over Language",
		description:
			"\x48\x49\x33 only supports Chinese/Japanese depending on region",
		type: "dropdown",
		values: ["English", "Japanese", "Chinese", "Korean"],
		onChange: async(newLang: string) => {
			await setOption<string>("voLanguage", newLang)
		},
	},
	{
		name: "Setting 2",
		type: "",
	},
	{
		name: "Verify Game Files",
		type: "button",
		onClick: async(game: Variants) => {

		}
	},
	{
		name: "Uninstall Game",
		type: "button",
		onClick: async(game: Variants) => {
			await remove(getActiveGameCode(game));
		}
	},
];
