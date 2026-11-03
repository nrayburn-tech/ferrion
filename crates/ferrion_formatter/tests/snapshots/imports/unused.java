package p;

import x.Unused;
import y.Used;
import static z.S.unused;
import static z.S.used;
import static z.S.Typ;

class T {
  Used u;
  Typ t;
  int i = used;
}
