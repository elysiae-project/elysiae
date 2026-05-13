import Modal from "../Modal";
import { AppModules, ModalHandle, Option, Variants } from "../../types";
import { forwardRef, useEffect, useState } from "preact/compat";
import { useApi } from "../../hooks/useApi";
import { useGame } from "../../hooks/useGame";
import {
	getActiveGameCode,
	getGameName,
	getGameSize,
} from "../../util/AppFunctions";
import Button from "../Button";
import { FileCheck, LucideIcon, RefreshCw, Trash } from "lucide-preact";
import {
	checkGameUpdate,
	downloadUpdate,
	isGameInstalled,
	isPreinstallAvailable,
	verifyGameIntegrity,
} from "../../lib/GameDownloader";
import { join } from "@tauri-apps/api/path";
import { remove } from "../../lib/Fs";
import Dropdown from "../Dropdown";
import { getOption, setOption } from "../../util/Settings";
import ToggleSwitch from "../ToggleSwitch";
import { getModuleVersion, updateWineComponent } from "../../lib/WineManager";

type GameOption = {
	icon: LucideIcon;
	text: string;
	action: (game: Variants) => Promise<void> | void;
}

const gameOptions: GameOption[] = [
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

const regularOptions: Option[] = [
	{
		name: "Preferred VO Language",
		type: "dropdown",
		labels: ["English", "Chinese", "Japanese", "Korean"],
		values: ["en", "cn", "jp", "kr"],
		getValue: async (): Promise<string> => {
			return await getOption<string>("voLanguage");
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
			await setOption("blockNotifications", !newValue);
		},
	},
];

const OptionRow = ({ option }: { option: (typeof regularOptions)[number] }) => {
	const [value, setValue] = useState<any>(null);

	useEffect(() => {
		option.getValue().then((res) => setValue(res));
	}, []);

	return (
		<div class="flex flex-row w-full justify-between items-center">
			<h1>{option.name}</h1>
			{option.type === "dropdown" ? (
				value !== null ? (
					<Dropdown
						width={250}
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
	useEffect(() => {
		getModuleVersion(componentName).then((res) => {
			if(res === null) {
				setVersion("Not Installed");
			}
			else setVersion(`Version ${res}`);
		});
	}, []);

	return (
		<div class="flex flex-row w-full">
			<div class="flex flex-col w-full justify-between">
				<h1>{componentName}</h1>
				<p>{version}</p>
			</div>
			<Button
			height={10}
			width={120}
				intent="primary"
				onClick={async () => updateWineComponent(componentName)}>
				Update
			</Button>
		</div>
	);
};

const DiskSize = () => {
	const { game } = useGame();
	const [size, setSize] = useState<string>("Calculating...");
	const [gameInstalled, setGameInstalled] = useState<boolean>(false);

	useEffect(() => {
		getGameSize(game).then((res) => {
			setGameInstalled(true);
			setSize(`${res.toFixed(2)}GB`);
		}).catch(() => {
			setGameInstalled(false);
			setSize("Calculating...");
		});
	}, [game]);
	if(!gameInstalled) return null;
	return <p class="text-sm">Size On Disk: {size}</p>;
};

const GameManagerButton = ({gameOption}: {gameOption: GameOption}) => {
	const { game } = useGame();
	const [gameInstalled, setGameInstalled] = useState<boolean>(false);

	useEffect(() => {
		isGameInstalled(game).then((res) => {
			setGameInstalled(res);
		})
	}, []);
	
	return <Button 
			intent="primary" 
			disabled={!gameInstalled}
			width={25} 
			height={25} 
			onClick={async() => {await gameOption.action(game)}}
			>
				{gameOption.text}
			</Button>

}

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
								<h1 class="text-md">{getGameName(game)}</h1>
								<DiskSize />
							</div>
						</div>
						<div class="flex flex-col justify-center h-auto mt-2.5 gap-y-2.5">
							{gameOptions.map((item) => {
								return (
									<GameManagerButton gameOption={item}/>
								);
							})}
						</div>
					</div>
					<div class="flex flex-col px-2 py-1.5 w-full">
						<div class="flex flex-col h-1/2">
							<h1 class="text-xl text-center">Wine Modules</h1>
							<div class="flex flex-col justify-between gap-y-2">
								{["wine", "dxvk", "jadeite"].map((wineComponent) => {
									return <ComponentInfo componentName={wineComponent as AppModules}/>
								})}
							</div>
						</div>
						<div class="h-1/2 flex flex-col gap-y-2.5 ">
							<h1 class="text-xl text-center">Settings</h1>
							{regularOptions.map((option, index) => {
								return <OptionRow key={index} option={option} />;
							})}
						</div>
					</div>
				</div>
			</Modal>
		);
	},
);

export default SettingsModal;
