export default function Titlebar() {
	return (
		<div
			data-tauri-drag-region
			class="bg-nap-titlebar border-t-2 border-r-2 border-l-2 border-nap-border rounded-tl-xl font-zzz px-5 py-3"
		>
			<h3 class="text-xl" data-tauri-drag-region>
				Yoohoo!
			</h3>
		</div>
	);
}
