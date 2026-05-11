import PreinstallButton from "./components/app/PreinstallButton.tsx";
import DownloadProgress from "./components/app/DownloadProgress.tsx";
import InstallerButton from "./components/app/InstallerButton.tsx";
import { DownloadProvider } from "./contexts/DownloadContext.tsx";
import { settingsDetails } from "./util/SettingsDetails.ts";
import { GameProvider } from "./contexts/GameContext.tsx";
import { ApiProvider } from "./contexts/ApiContext.tsx";
import Background from "./components/Background.tsx";
import Sidebar from "./components/app/Sidebar.tsx";
import Titlebar from "./components/Titlebar.tsx";
import { cva } from "class-variance-authority";
import { Info, Settings } from "lucide-preact";
import Button from "./components/Button.tsx";
import { useGame } from "./hooks/useGame.ts";
import Modal from "./components/Modal.tsx";
import { useApi } from "./hooks/useApi.ts";
import { useState } from "preact/hooks";
import { Variants } from "./types";
import "./App.css";

const theme = cva("h-full w-full overflow-hidden", {
	variants: {
		intent: {
			[Variants.BH3]: "bg-bh3-bg font-bh3-hkrpg rounded-b-xl text-white",
			[Variants.HK4E]: "bg-hk4e-bg font-hk4e text-black",
			[Variants.HKRPG]: "bg-hkrpg-bg font-bh3-hkrpg rounded-b-xs text-black",
			[Variants.NAP]:
				"bg-nap-bg font-nap rounded-br-xl border-nap-border text-white",
		},
	},
});

function App() {
	const { game } = useGame();
	const { graphics } = useApi();
	const [settingsOpen, setSettingsOpen] = useState<boolean>(false);

	return (
		<div class="flex h-screen w-screen flex-col gap-0 text-white">
			<Titlebar />

			<div class={theme({ intent: game })}>
				<div class="relative h-full w-full">
					{graphics ? <Background /> : <></>}
					<Modal
						onOpenUpdate={() => setSettingsOpen(false)}
						title="Settings"
						open={settingsOpen}>
						{settingsDetails.map((setting) => {
							return (
								<div class="justify-apart flex h-full w-full flex-col gap-y-2.5">
									<div class="justify-left flex flex-row items-center gap-x-1">
										<p>{setting.name}</p>
										{typeof setting.description !== "undefined" ? (
											<Info size={15} />
										) : null}
									</div>
									<div>
										{(() => {
											if (setting.type === "dropdown") {
											} else if (setting.type === "boolean") {
											} else if (setting.type === "button") {
											}
										})()}
									</div>
								</div>
							);
						})}
					</Modal>

					<div class="absolute inset-0 z-10 flex w-full flex-row items-end justify-end gap-x-3 px-15 py-10">
						{/* Page content */}
						<DownloadProgress />
						<PreinstallButton />
						<Button
							intent="secondary"
							onClick={() => {
								setSettingsOpen(true);
							}}
							iconButton>
							<Settings className="-m-1 leading-0" />
						</Button>
						<InstallerButton />
					</div>
					<Sidebar />
				</div>
			</div>
		</div>
	);
}

export default function AppWrapper() {
	return (
		<GameProvider>
			<ApiProvider>
				<DownloadProvider>
					<App />
				</DownloadProvider>
			</ApiProvider>
		</GameProvider>
	);
}
