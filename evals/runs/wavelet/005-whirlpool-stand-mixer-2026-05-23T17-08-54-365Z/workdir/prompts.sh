#!/bin/bash
export PATH="$PWD/.bin:$PATH"
LOCK="50mm full-frame, medium-close framing, warm morning window light from camera-left at 3200K, deep cream and charcoal palette with KitchenAid red as the single saturated accent, gentle film grain, A24 domestic-warm grade, locked 9:16 portrait"

declare -a P=(
  "Extreme close-up: a soft pale ball of bread dough drops in slow motion into the polished steel mixing bowl of an empire-red KitchenAid Artisan stand mixer, dough lands with a soft squish, tiny puff of flour dust, ${LOCK}"
  "A woman's hand pulls the chrome tilt-head lever of an empire-red KitchenAid Artisan stand mixer and tips the head down, the metal latch settles into place, marble counter background, ${LOCK}"
  "Macro shot inside the bowl: the polished steel spiral dough hook rotates slowly, pale dough catches against the bowl wall and folds over itself in smooth rhythmic turns, ${LOCK}"
  "Wide reveal: an empire-red KitchenAid Artisan stand mixer sits centered on a white marble counter in a sunlit kitchen, the mixer head is down and the bowl is locked in place, soft camera push-in, ${LOCK}"
  "Close-up: two hands shape a ball of proofed bread dough on a flour-dusted wooden board, the empire-red KitchenAid Artisan mixer is softly out of focus behind, ${LOCK}"
  "A finished round crusty sourdough loaf sits on a cast-iron tray fresh from the oven, steam rising in the warm morning light, hands enter frame and set the tray down on a wooden counter, ${LOCK}"
)

for i in 0 1 2 3 4 5; do
  echo "=== shot $i ===" >> shots/log
  wavelet shot txt2vid "${P[$i]}" \
    --duration 4 --aspect 9:16 --max-cost 0.55 \
    --out "shots/shot-${i}.mp4" --pretty >> "shots/shot-${i}.json" 2>> shots/log &
done
wait
echo "ALL DONE"
