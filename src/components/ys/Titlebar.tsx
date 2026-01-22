import YsTitlebarButtons from "./TitlebarButtons";

export default function YsTitlebar({
	onClose,
	onMinimize,
}: {
	onClose: () => void;
	onMinimize: () => void;
}) {
	return (
		<div
			data-tauri-drag-region
			class="bg-ys-titlebar font-ys h-auto min-w-full p-1"
		>
			<div
				data-tauri-drag-region
				class="flex flex-row justify-between items-center border border-[#505869] bg-transparent px-5 py-1.5"
			>
				<h3 class="text-xl text-center" data-tauri-drag-region>
					Yoohoo!
				</h3>
				<YsTitlebarButtons close={onClose} minimize={onMinimize} />
			</div>
		</div>
	);
}
