// A header above the first import.
package p.q;
// Directly above the first import.
import z.Z; // Trailing Z.
import static a.B.c;
/* Above Y. */ import y.Y;

// A floating header.

import p.q.Own;
import /* inside */ x.X;
import x.X;
import java.lang.String;
import module java.base;
import a.*;
import u.Unused; /* trailing */ /* twice */
import u.Doc;
// After the last import.
/** {@link Doc#m(Y)} @see X */
class T<Z> { Z z; Y y; Own o; void f() { c(); } }
