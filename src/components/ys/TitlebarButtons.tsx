import { Shrink, Minus } from "lucide-preact";

export default function YsTitlebarButtons({
		close, 
		minimize
	}: {
		close: () => void, 
		minimize: () => void
	}) {
	const content = [
		{
			icon: Shrink,
			action: close,
		},
		{
			icon: Minus,
			action: minimize,
		}
	] as const;

	return (
		<div class="flex flex-row-reverse gap-2 items-center justify-center overflow-visible">
			{content.map((item, key) => {
				const Icon = item.icon;
				return (
					<div class="border-5 border-[#888d8e] bg-[#ece5d8] hover:border-[#00000000]  hover:drop-shadow-xs hover:drop-shadow-[#fdfdfeAA] 
						active:bg-[#9a947f] active:border-[#00000000] p-2 rounded-full">
						<Icon key={key} color="#4b5366" onClick={item.action}/>
					</div>
				);
			})}
		</div>
	);
}
