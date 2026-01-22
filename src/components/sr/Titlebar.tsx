export default function SrTitlebar({
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
			class="bg-sr-titlebar rounded-t-xs titlebar-hsr-noise font-hsr-hi3 border-sr-border flex flex-row justify-between border-l-2 border-r-2 border-t-2 px-5 py-3"
		>
			<h3 class="text-xl" data-tauri-drag-region>
				Yoohoo!
			</h3>
		</div>
	);
}
