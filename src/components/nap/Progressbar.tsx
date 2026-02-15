export default function NapProgressbar({ progress }: { progress: number }) {
	return (
		<div class="border-2 border-[#212222] bg-[#262626] h-5 rounded-full">
			<div
				style={{ width: `${progress}%` }}
				class="bg-linear-to-r from-[#4766fe] from-10% via-[#529aff] via-60% to-[#5ec6ff] h-full transition-all duration-300 rounded-full"
			></div>
		</div>
	);
}
