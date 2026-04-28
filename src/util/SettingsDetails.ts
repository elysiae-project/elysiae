import { setOption } from "./Settings";

export const settingsDetails: any[] = [
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
		type: "boolean",
		onChange: async (permissionUpdate: boolean) => {
			await setOption("blockNotifications", permissionUpdate);
		},
	},
];
