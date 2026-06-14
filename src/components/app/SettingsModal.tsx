import { appDataDir, join } from "@tauri-apps/api/path";
import { openPath } from "@tauri-apps/plugin-opener";
import {
	FileCheck,
	Folder,
	type LucideIcon,
	RefreshCw,
	Trash,
} from "lucide-preact";
import { forwardRef, useEffect, useState } from "preact/compat";
import { useApi } from "../../hooks/useApi";
import { useDownload } from "../../hooks/useDownload";
import { useGame } from "../../hooks/useGame";
import { getDirSize, remove } from "../../lib/Fs";
import {
	checkGameUpdate,
	downloadUpdate,
	isGameInstalled,
	verifyGameIntegrity,
} from "../../lib/GameDownloader";
import {
	getModuleVersion,
	moduleTagsMatch,
	updateProtonComponent,
} from "../../lib/ProtonManager";
import { getOption, setOption } from "../../lib/Settings";
import {
	variantToGameCode,
	variantToGameName,
} from "../../lib/VariantConverter";
import type { AppModules, ModalHandle, Option, Variants } from "../../types";
import Button from "../Button";
import Dropdown from "../Dropdown";
import Modal from "../Modal";
import ToggleSwitch from "../ToggleSwitch";

type GameOption = {
	icon: LucideIcon;
	action: (game: Variants) => Promise<void> | void;
};

const gameOptions: GameOption[] = [
	{
		icon: Trash,
		action: async (game: Variants) => {
			// TODO: Add confirmation modal
			const gameCode = variantToGameCode[game];
			const path = await join("games", gameCode);
			await remove(path);
		},
	},
	{
		icon: FileCheck,
		action: async (game: Variants) => {
			await verifyGameIntegrity(game);
		},
	},
	{
		icon: RefreshCw,
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
	{
		icon: Folder,
		action: async (game: Variants) => {
			const folder = await join(
				await appDataDir(),
				"games",
				variantToGameCode[game],
			);
			await openPath(folder);
		},
	},
];

const options: Option[] = [
	{
		name: "Default Voice-Over Language",
		type: "dropdown",
		labels: ["English", "Chinese", "Japanese", "Korean"],
		values: ["en", "cn", "jp", "kr"],
		getValue: async (): Promise<string> => {
			return (await getOption<string>("voLanguage")) ?? "en";
		},
		setValue: async (newLang: string): Promise<void> => {
			await setOption("voLanguage", newLang);
		},
	},
	{
		// TODO: Rename blockNotifications to allowNotifications for clarity
		name: "Allow Notifications",
		type: "boolean",
		getValue: async (): Promise<boolean> => {
			return !(await getOption("blockNotifications"));
		},
		setValue: async (newValue: boolean): Promise<void> => {
			// No need to request permission as:
			// 1. Notifications are enabled by default
			// 2. If user rejects notifications when they first pop up, the setting value will be changes to false
			// 3. Once the notifications are accepted on the DE side, the setting in the json effectiely takes over. If the user rejects notifications on the request, then re-enables them, they will get another request, and the cycle repeats. once the permission is accepted once though, there will be no permission popup again
			await setOption("blockNotifications", !newValue);
		},
	},
];

const OptionRow = ({ option }: { option: (typeof options)[number] }) => {
	// biome-ignore lint/suspicious/noExplicitAny: The best solution for this component is to just use any. I haven't found any other good intuitive solutions
	const [value, setValue] = useState<any>(null);

	useEffect(() => {
		option.getValue().then((res) => setValue(res));
	}, []);

	return (
		<div class="flex w-full flex-row items-center justify-between">
			<h1>{option.name}</h1>
			{option.type === "dropdown" ? (
				value !== null ? (
					<Dropdown
						width={12}
						height={2.25}
						labels={option.labels}
						values={option.values}
						initialValue={value}
						onChangeAction={async (newValue: string) => {
							await option.setValue(newValue);
						}}
					/>
				) : (
					<div style={{ width: 250 }} />
				)
			) : null}
			{option.type === "boolean" ? (
				value !== null ? (
					<ToggleSwitch
						size="sm"
						height={2.15}
						width={5.25}
						startActive={value}
						onClick={async (newValue) => {
							await option.setValue(newValue);
						}}
					/>
				) : (
					<div style={{ width: 120 }} />
				)
			) : null}
		</div>
	);
};

const ComponentInfo = ({ componentName }: { componentName: AppModules }) => {
	const [version, setVersion] = useState<string>("");
	const [updateAvailable, setUpdateAvailable] = useState<boolean>(false);
	const { setProtonSetupProgress } = useDownload();

	useEffect(() => {
		getModuleVersion(componentName).then((res) => {
			if (res === null) {
				setVersion("Not Installed");
				moduleTagsMatch(componentName).then((res) => {
					setUpdateAvailable(!res);
				});
			} else setVersion(`${res}`);
		});
	}, []);

	return (
		<div class="flex flex-row items-center justify-between">
			<div class="flex flex-col text-left">
				<h1 class="text-[1.10rem]">
					{String(componentName).charAt(0).toUpperCase() +
						String(componentName).slice(1)}
				</h1>
				<p class="text-[0.85rem]">{version}</p>
			</div>
			<Button
				height={2.3}
				width={6.5}
				size="sm"
				variant={updateAvailable ? "primary" : "secondary"}
				disabled={!updateAvailable}
				onClick={() => {
					updateProtonComponent(componentName, (event) => {
						setProtonSetupProgress(event);
					});
					setUpdateAvailable(false);
				}}
			>
				<p class="text-[1rem]">Update</p>
			</Button>
		</div>
	);
};

const DiskSize = () => {
	const { game } = useGame();
	const [size, setSize] = useState<string>("Calculating...");
	const [gameInstalled, setGameInstalled] = useState<boolean>(false);

	useEffect(() => {
		getDirSize(game)
			.then((res) => {
				setGameInstalled(true);
				setSize(`${res.toFixed(2)}GB`);
			})
			.catch(() => {
				setGameInstalled(false);
				setSize("Calculating...");
			});
	}, [game]);
	return (
		<p class="text-sm">
			{gameInstalled ? `Size On Disk: ${size}` : `Not Installed`}
		</p>
	);
};

const GameManagerButton = ({ gameOption }: { gameOption: GameOption }) => {
	const { game } = useGame();
	const [gameInstalled, setGameInstalled] = useState<boolean>(false);
	const Icon = gameOption.icon;

	useEffect(() => {
		isGameInstalled(game).then((res) => {
			setGameInstalled(res);
		});
	}, []);

	return (
		<Button
			variant={gameInstalled ? "primary" : "secondary"}
			disabled={!gameInstalled}
			size="xs"
			width={2.1}
			height={2.1}
			onClick={async () => {
				await gameOption.action(game);
			}}
		>
			<Icon />
		</Button>
	);
};

export const SettingsModal = forwardRef<ModalHandle>((_, ref) => {
	const { branding } = useApi();
	const { game } = useGame();

	return (
		<Modal ref={ref} width={750} height={450}>
			<div class="flex h-full w-full flex-col gap-y-5 overflow-y-scroll py-2.5">
				<div class="flex flex-row justify-between">
					<div class="flex flex-row gap-x-2.5">
						<img
							class="rounded-lg"
							width={52}
							height={52}
							alt=""
							src={branding?.[game].icon}
						/>
						<div class="flex flex-col text-left justify-center">
							<h1>{variantToGameName[game]}</h1>
							<DiskSize />
						</div>
					</div>
					<div class="flex flex-row-reverse items-center gap-x-2.5">
						{gameOptions.map((option) => (
							<GameManagerButton gameOption={option} />
						))}
					</div>
				</div>
				<div>
					<h1 class="mb-2.5 text-xl">Options</h1>
					<div class="flex flex-col gap-y-3">
						{options.map((option) => (
							<OptionRow option={option} />
						))}
					</div>
				</div>
				<div class="mb-2.5">
					<h1 class="mb-2.5 text-xl">Modules</h1>
					<div class="flex flex-col gap-y-3">
						{["proton", "jadeite"].map((item) => (
							<ComponentInfo componentName={item as AppModules} />
						))}
					</div>
				</div>
			</div>
		</Modal>
	);
});

export default SettingsModal;
