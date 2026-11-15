import { useEffect } from 'react';
import { useAuthority } from '../../lib/authority';
import { Button, toast } from '../../ui';

/** The owner takes back this Console's power to act for people; their passkey signs it. */
export function ConsoleAccess({ node }: { node: string }) {
  const authority = useAuthority();
  const { prepare } = authority;
  // The challenge is ready before the click, so the passkey prompt opens from the gesture.
  useEffect(() => prepare({ kind: 'revoke_console', device: node }), [node, prepare]);
  const revoke = async () => {
    if (authority.signing) return;
    if (await authority.confirm()) toast.success('This Console no longer acts for anyone.');
  };
  return (
    <div className="flex w-full min-w-0 flex-wrap items-center gap-x-4 gap-y-3 border border-line px-4 py-3.5">
      <div className="min-w-[12rem] flex-1">
        <p className="m-0 text-[14px] text-fg">Console access</p>
        <p className="m-0 mt-0.5 text-[13px] text-fg-2">This Console can make changes for signed-in admins.</p>
      </div>
      <Button size="sm" busy={authority.signing} onClick={() => void revoke()}>Revoke access</Button>
      {authority.error && <p role="alert" className="m-0 w-full text-[13px] text-hot">{authority.error}</p>}
    </div>
  );
}
