import TitlebarButtons from "./TitlebarButtons";

export default function YsTitlebar({
	onClose,
	onToggleMaximize,
	onMinimize,
}: {
	onClose: () => void;
	onToggleMaximize: () => void;
	onMinimize: () => void;
}) {
	return (
		<div
			data-tauri-drag-region
			class="bg-ys-titlebar font-ys h-auto min-w-full p-1"
		>
			<div
				data-tauri-drag-region
				class="flex flex-row justify-between border border-[#505869] bg-transparent px-5 py-3"
			>
				<h3 class="text-xl" data-tauri-drag-region>
					Yoohoo!
				</h3>
				<TitlebarButtons />
			</div>
		</div>
	);
}
