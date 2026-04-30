import Modal from "../Modal";
import { ModalHandle, Variants } from "../../types";
import { forwardRef } from "preact/compat";
import { useApi } from "../../hooks/useApi";
import { useGame } from "../../hooks/useGame";
import { getActiveGameCode, getGameName } from "../../util/AppFunctions";
import Button from "../Button";
import { FileCheck, RefreshCw, Trash } from "lucide-preact";
import {
	checkGameUpdate,
	downloadUpdate,
	isPreinstallAvailable,
	verifyGameIntegrity,
} from "../../lib/GameDownloader";
import { join } from "@tauri-apps/api/path";
import { remove } from "../../lib/Fs";
import Dropdown from "../Dropdown";
import { getOption, setOption } from "../../util/Settings";

const gameOptions = [
	{
		icon: Trash,
		text: "Uninstall",
		action: async (game: Variants) => {
			// TODO: Add confirmation modal
			const gameCode = getActiveGameCode(game);
			const path = await join("games", gameCode);
			await remove(path);
		},
	},
	{
		icon: FileCheck,
		text: "Verify",
		action: async (game: Variants) => {
			await verifyGameIntegrity(game);
		},
	},
	{
		icon: RefreshCw,
		text: "Check for Updates",
		action: async (game: Variants) => {
			const res = await checkGameUpdate(game);
			if (res !== null) {
				if (res.preinstallAvailable) {
					await downloadUpdate(game, true);
				} else {
					await downloadUpdate(game, false);
				}
			}
		},
	},
];

const regularOptions = [
	{
		name: "Preferred VO Language",
		type: "dropdown",
		values: ["English", "Chinese", "Japanese", "Korean"],
		getValue: async (): Promise<number> => {
			const unparsed: "en" | "cn" | "jp" | "kr" = await getOption("voLanguage");
			switch (unparsed) {
				case "en":
					return 0;
				case "cn":
					return 1;
				case "jp":
					return 2;
				case "kr":
					return 3;
			}
		},
		setValue: async (newLang: string): Promise<void> => {
			await setOption(
				"voLanguage",
				newLang === "English"
					? "en"
					: newLang === "Chinese"
						? "cn"
						: newLang === "Japanese"
							? "jp"
							: "kr",
			);
		},
	},
];

export const SettingsModal = forwardRef<ModalHandle>(
	function SettingsModal(_, ref) {
		const { branding } = useApi();
		const { game } = useGame();

		return (
			<Modal ref={ref} title="Elysiae Settings" width={900} height={450}>
				<div class="flex flex-row w-full min-h-112.5">
					<div class="min-w-[35%] px-2 py-1.5 border-r-2 border-gray-500">
						<div class="flex flex-row gap-x-2.5 border-b-2 py-1.5">
							<div class="border-2 rounded-sm">
								<img
									width={60}
									height={60}
									alt=""
									src={branding?.[game].icon}
								/>
							</div>
							<div class="flex flex-col justify-center">
								<h1 class="text-sm">{getGameName(game)}</h1>
								<h2 class="text-sm">Size On Disk: xxGB</h2>
							</div>
						</div>
						<div class="flex flex-row justify-center h-auto mt-2.5 gap-x-2.5">
							{gameOptions.map((item) => {
								return (
									<Button
										intent="primary"
										width={25}
										height={25}
										onClick={async () => {
											await item.action(game);
										}}>
										s{" "}
									</Button>
								);
							})}
						</div>
					</div>
					<div class="min-w-[65%] px-2 py-1.5">
						<div class="flex flex-row justify-between items-center">
							<h1>Preferred VO Language</h1>
							<Dropdown
								width={250}
								labels={["English", "Japanese", "Chinese", "Korean"]}
								initialIndex={0}
								onChangeAction={async (newLanguage: string) => {
									await setOption("voLanguage", newLanguage);
								}}
							/>
						</div>
					</div>
				</div>
			</Modal>
		);
	},
);

export default SettingsModal;
