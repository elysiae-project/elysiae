import { onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { warn } from "@tauri-apps/plugin-log";
import { Variants } from "../types";
import { runGame } from "./GameDownloader";

export const startListening = async () => {
	await onOpenUrl((uris) => {
		console.log(`URI(s) opened: ${uris}`);
		const commands = uris.map((uri) => uri.split("://")[1]);
		handleURIs(commands);
	});
};

const handleURIs = (uriCommand: string[]) => {
	uriCommand.map(async (command) => {
		switch (command) {
			case "run-bh3":
				await runGame(Variants.BH3);
				break;
			case "run-hk4e":
				await runGame(Variants.HK4E);
				break;
			case "run-hkrpg":
				await runGame(Variants.HKRPG);
				break;
			case "run-nap":
				await runGame(Variants.NAP);
				break;
			default:
				warn(
					`handleURIs: the URI command ${command} is not recognized by Elysiae`,
				);
				break;
		}
	});
};
