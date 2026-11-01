import { describe, expect, it } from 'vitest';
import { href, parse, setNetworks, slug } from './router';

const HOME = 'aaaaaaaa11111111';
const WORK = 'bbbbbbbb22222222';
const HOME_2 = 'cccccccc33333333';

describe('router (spec §4.1)', () => {
  it('addresses networks by name and keeps ids inside the page', () => {
    setNetworks([
      { id: HOME, name: 'home' },
      { id: WORK, name: 'work' },
    ]);
    expect(parse('/n/home/devices')).toEqual({ name: 'devices', net: HOME });
    expect(parse('/n/work/devices/nas-id')).toEqual({ name: 'device', net: WORK, dev: 'nas-id' });
    expect(href({ name: 'activity', net: WORK })).toBe('/n/work/activity');
    expect(parse('/n/elsewhere')).toEqual({ name: 'not_found' });
    expect(parse('/n/home/inbox')).toEqual({ name: 'not_found' });
    expect(parse('/')).toEqual({ name: 'root' });
  });

  it('falls back to a short id when two networks share a name', () => {
    setNetworks([
      { id: HOME, name: 'home' },
      { id: HOME_2, name: 'home' },
    ]);
    expect(slug(HOME)).toBe('aaaaaaaa');
    expect(href({ name: 'overview', net: HOME_2 })).toBe('/n/cccccccc');
    expect(parse('/n/cccccccc/exits')).toEqual({ name: 'exits', net: HOME_2 });
    expect(parse('/n/home')).toEqual({ name: 'not_found' });
  });
});
