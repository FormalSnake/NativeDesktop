// Built by build.test.ts: data's main entry stays an external import, its
// Solid binding (which imports solid-js) is bundled.
import { openDatabase } from "@nativedesktop/data";
import { createQuery } from "@nativedesktop/data/solid";

console.log(typeof openDatabase, typeof createQuery);
