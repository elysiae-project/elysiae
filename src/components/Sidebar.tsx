import { useApi } from "../hooks/useApi";
import { useGame } from "../hooks/useGame";

export default function Sidebar() {
	const { game, setGame } = useGame();
	const { branding } = useApi();

	return (
		<div class="bg-linear-to-b absolute bottom-0 left-0 top-0 z-20 flex flex-col items-center gap-4 from-black/50 to-black/30 p-4">
			{branding &&
				Object.entries(branding).map(([key, data]) => (
					<button
						key={key}
						class="relative h-12 w-12 cursor-pointer rounded-lg border-white transition-transform hover:scale-105"
						style={{
							borderWidth: game === +key ? "0.125rem" : "",
						}}
						onClick={() => setGame(+key)}
					>
						<img class="absolute inset-0" src={data.icon} alt="" />
					</button>
				))}
		</div>
	);
}
