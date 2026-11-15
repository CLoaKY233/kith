import { SegmentedControl, SegmentedControlIndicator, SegmentedControlItem } from '@ui-sdk/react/segmented-control';
import { ListIcon, NetworkIcon, PlusIcon } from 'lucide-react';
import { useRoute } from '../../lib/router';
import { Button } from '../../ui';

/** Header actions shared by the Devices list and map: the view switch and "Add device". */
export function DevicesActions({ view }: { view: 'list' | 'map' }) {
  const { go } = useRoute();
  return (
    <>
      <SegmentedControl
        aria-label="View"
        value={view}
        onValueChange={(v) => v && v !== view && go(v === 'map' ? '/devices?view=map' : '/devices')}
      >
        <SegmentedControlIndicator />
        <SegmentedControlItem value="list" className="gap-1.5">
          <ListIcon className="size-3.5" aria-hidden="true" /> List
        </SegmentedControlItem>
        <SegmentedControlItem value="map" className="gap-1.5">
          <NetworkIcon className="size-3.5" aria-hidden="true" /> Map
        </SegmentedControlItem>
      </SegmentedControl>
      <Button variant="primary" icon={<PlusIcon />} onClick={() => go('/settings/network')}>
        Add device
      </Button>
    </>
  );
}
