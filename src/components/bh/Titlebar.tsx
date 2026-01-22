export default function BhTitlebar({
	onClose,
	onMinimize,
}: {
	onClose: () => void;
	onMinimize: () => void;
}) {
	return (
		<div
			data-tauri-drag-region
			class="bg-bh-titlebar font-bh-sr flex h-auto w-full flex-row justify-between rounded-t-md px-5 py-3">
			<h3 class="text-xl" data-tauri-drag-region>
				Yoohoo!
			</h3>
		</div>
	);
}
