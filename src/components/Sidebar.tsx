import { useApi } from "../hooks/useApi";

export default function Sidebar() {
	const { graphics } = useApi();

	return (
		<div class="bg-linear-to-r fixed bottom-0 left-0 top-0 flex w-16 flex-col items-center from-black/50 to-transparent">
			{graphics &&
				Object.entries(graphics).map(([key, data]) => (
					<button key={key} class="relative h-8 w-8 rounded">
						<img class="absolute inset-0" src={data.icon} alt="" />
					</button>
				))}
		</div>
	);
}
