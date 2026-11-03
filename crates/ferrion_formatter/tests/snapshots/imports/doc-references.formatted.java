package p;

import static u.S.member;

import u.Exc;
import u.Field;
import u.Gen;
import u.Link;
import u.LinkArr;
import u.LinkM;
import u.LinkParam;
import u.LinkPlain;
import u.Provides;
import u.QualFirst;
import u.See;
import u.Thr;
import u.Uses;
import u.Value;

/** {@link Before} */
/**
 * Prose. {@link Link} {@link LinkM#m} {@linkplain LinkPlain text} {@link #m(LinkParam)} {@link
 * LinkArr[]} {@link String LinkLabel} {@link Gen<String>} {@link QualFirst.Inner} {@link
 * a.QualLast} {@value Value#X} {@value ValueOnly} {@code Code} {@literal Literal} {@link member}
 * {@link called()}
 *
 * @param <Q> ParamDoc
 * @see See
 * @see String SeeLabel
 * @throws Thr when
 * @exception Exc when
 * @uses Uses
 * @provides Provides
 */
class T<Q> {
  /** {@link Field} */
  int f;

  void m() {
    /** {@link InBody} */
    int x;
  }

  /** {@link Dangling} */
}
