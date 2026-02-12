export default function NapTitlebar({ children }: { children: any }) {
	return (
		<div
			data-tauri-drag-region
			class="bg-nap-titlebar border-nap-border font-nap rounded-tl-xl border-l-2 border-r-2 border-t-2 px-5 py-3"
		>
			<h3 class="text-xl" data-tauri-drag-region>
				Yoohoo!
			</h3>
			{children}
		</div>
	);
}
