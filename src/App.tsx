import "./App.css";
import { useGame } from "./hooks/useGame.ts";
import Titlebar from "./components/Titlebar.tsx";
import { cva } from "class-variance-authority";
import { Variants } from "./types";
import { useApi } from "./hooks/useApi.ts";
import Sidebar from "./components/Sidebar.tsx";
import { ApiProvider } from "./contexts/ApiContext.tsx";
import { GameContext, GameProvider } from "./contexts/GameContext.tsx";

const theme = cva("h-full w-full overflow-hidden", {
	variants: {
		intent: {
			[Variants.BH]: "bg-bh-bg font-bh-sr rounded-b-xl text-white",
			[Variants.YS]: "bg-ys-bg font-ys text-black",
			[Variants.SR]:
				"bg-sr-bg font-bh-sr rounded-b-xs border border-[#393939] text-black",
			[Variants.NAP]:
				"bg-nap-bg font-nap rounded-br-xl border-b-2 border-r-2 border-l-2 border-nap-border text-white",
		},
	},
});

function Background() {}

function App() {
	const { game } = useGame();
	const { graphics } = useApi();

	return (
		<div class="flex h-screen w-screen flex-col gap-0 text-white">
			<Titlebar />

			<div class={theme({ intent: game })}>
				{graphics ? (
					<div class="relative h-full w-full">
						<video
							class="absolute inset-0 h-full w-full object-cover"
							src={graphics[game].backgroundVideo}
							autoplay
							loop
							muted
						></video>
						<img
							class="absolute inset-0 h-full w-full object-cover"
							src={graphics[game].backgroundVideoOverlay}
							alt=""
						/>

						<div class="absolute inset-0 z-10 flex flex-col items-center justify-center text-center">
							{/* Page content */}
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
				<App />
			</ApiProvider>
		</GameProvider>
	);
}
