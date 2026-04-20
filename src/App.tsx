import "./App.css";
import Button from "./components/Button.tsx";
import Background from "./components/Background.tsx";
import Sidebar from "./components/Sidebar.tsx";
import Titlebar from "./components/Titlebar.tsx";
import { useGame } from "./hooks/useGame.ts";
import { cva } from "class-variance-authority";
import { Variants } from "./types";
import { useApi } from "./hooks/useApi.ts";
import { ApiProvider } from "./contexts/ApiContext.tsx";
import { GameProvider } from "./contexts/GameContext.tsx";
import { useState } from "preact/hooks";
import { Info, Settings } from "lucide-preact";
import { settingsDetails } from "./util/SettingsDetails.ts";
import Modal from "./components/Modal.tsx";
import DownloadProgress from "./components/app/DownloadProgress.tsx";
import PreinstallButton from "./components/app/PreinstallButton.tsx";
import InstallerButton from "./components/app/InstallerButton.tsx";
import Dropdown from "./components/Dropdown.tsx";
import { setOption } from "./util/Settings.ts";
import { DownloadProvider } from "./contexts/DownloadContext.tsx";

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
				{graphics ? (
					<div class="relative h-full w-full">
						<Background />
						<Modal
							onOpenUpdate={() => setSettingsOpen(false)}
							title="Settings"
							open={settingsOpen}>
							{settingsDetails.map((setting) => {
								return (
									<div class="flex flex-col justify-apart w-full h-full gap-y-2.5">
										<div class="flex flex-row items-center justify-left gap-x-1">
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

						<div class="absolute inset-0 z-10 flex flex-row items-end justify-end px-15 py-10 w-full gap-x-3">
							{/* Page content */}
							<DownloadProgress />
							<PreinstallButton />
							<Button
								intent="secondary"
								onClick={() => {
									setSettingsOpen(true);
								}}
								iconButton>
								<Settings className="leading-0 -m-1" />
							</Button>
							<InstallerButton />
						</div>
						<Sidebar />
					</div>
				) : (
					<div class="flex h-full w-full flex-col items-center justify-center text-center">
						<h2 class="text-6xl">Loading...</h2>
					</div>
				)}
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
