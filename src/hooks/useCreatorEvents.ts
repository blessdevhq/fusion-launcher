'use client';

import { useEffect } from 'react';
import { subscribeCreatorJobProgress, subscribeInputState } from '@/lib/creatorApi';
import { creatorError, useCreatorStore } from '@/stores/creatorStore';

/** Lives with the shell: navigating away never cancels jobs or drops their ordered history. */
export function useCreatorEvents() {
  useEffect(() => {
    let active = true;
    const jobs = subscribeCreatorJobProgress((event) => {
      if (active) useCreatorStore.getState().mergeJobProgress(event);
    });
    const input = subscribeInputState((event) => {
      if (active) useCreatorStore.getState().mergeInputState(event);
    });
    void jobs.catch((error) => { if (active) useCreatorStore.setState({ jobError: creatorError(error) }); });
    void input.catch((error) => { if (active) useCreatorStore.setState({ inputError: creatorError(error), inputStatus: 'error' }); });
    return () => {
      active = false;
      void jobs.then((unlisten) => unlisten(), () => {});
      void input.then((unlisten) => unlisten(), () => {});
    };
  }, []);
}
