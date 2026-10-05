<script lang="ts">
  import Segmented from './Segmented.svelte';
  import Slider from './Slider.svelte';
  import { t } from './i18n';
  import { store } from './store.svelte';
  import type { GateSensitivity } from './types';

  let p = $derived(store.state!.settings.noise.pause);
  const sensitivities: GateSensitivity[] = ['low', 'normal', 'high'];
  // Depth choices in dB; -60 means "silent".
  const depths = ['-60', '-24', '-12'] as const;

  let holdDrag = $state<number | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  function setHold(v: number) {
    holdDrag = v;
    clearTimeout(timer);
    timer = setTimeout(() => {
      store.apply({ noise: { pause: { hold_ms: v } } });
      setTimeout(() => (holdDrag = null), 600);
    }, 150);
  }
  let depth = $derived(p.depth_db <= -60 ? '-60' : p.depth_db <= -18 ? '-24' : '-12');
</script>

<div class="pause-settings">
  <Slider
    label={t('pause.hold')}
    min={100}
    max={1500}
    step={50}
    value={holdDrag ?? p.hold_ms}
    format={(v) => `${v} ms`}
    onchange={setHold}
  />
  <small>{t('pause.hold_sub')}</small>
  <span class="lbl">{t('pause.sensitivity')}</span>
  <Segmented
    label={t('pause.sensitivity')}
    options={sensitivities.map((v) => ({ value: v, label: t(`pause.sens.${v}`) }))}
    value={p.sensitivity}
    onchange={(v) => store.apply({ noise: { pause: { sensitivity: v } } })}
  />
  <small>{t('pause.sensitivity_sub')}</small>
  <span class="lbl">{t('pause.depth')}</span>
  <Segmented
    label={t('pause.depth')}
    options={depths.map((v) => ({ value: v, label: t(`pause.depth.${v}`) }))}
    value={depth}
    onchange={(v) => store.apply({ noise: { pause: { depth_db: Number(v) } } })}
  />
</div>

<style>
  .pause-settings {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 10px;
    padding: 12px;
    border-radius: 12px;
    border: 1px solid var(--line);
    font-size: 13.5px;
  }
  small {
    color: var(--sub);
    font-size: 12.5px;
    margin-top: -2px;
  }
  .lbl {
    margin-top: 6px;
  }
</style>
