"use client";

import { useEffect, useState } from "react";

export function BuilderCount() {
  const [count, setCount] = useState(7834);

  useEffect(() => {
    let timeout: ReturnType<typeof setTimeout>;

    function scheduleIncrement() {
      const delay = 4000 + Math.random() * 6000;
      timeout = setTimeout(() => {
        setCount((current) => current + 1);
        scheduleIncrement();
      }, delay);
    }

    scheduleIncrement();
    return () => clearTimeout(timeout);
  }, []);

  return (
    <span className="builderCount">
      <span className="builderNumber" key={count}>
        {count.toLocaleString()}+
      </span>
      <span>builders choosing GitMesh</span>
    </span>
  );
}
