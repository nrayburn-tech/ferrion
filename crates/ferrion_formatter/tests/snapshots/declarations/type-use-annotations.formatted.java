import org.jspecify.annotations.Nullable;
import org.checkerframework.checker.nullness.qual.NonNull;
import javax.annotation.CheckForNull;

abstract class TypeUseAnnotations {
  @Nullable String a() {
    return null;
  }

  @NonNull String b() {
    return "";
  }

  @CheckForNull
  String c() {
    return null;
  }

  @Nullable
  public String d() {
    return null;
  }

  @Override
  @Nullable String e() {
    return null;
  }

  @Nullable
  @Override
  String f() {
    return null;
  }

  @Nullable <T> T g() {
    return null;
  }

  @Nullable void h() {}

  @Nullable
  TypeUseAnnotations() {}

  @Nullable
  abstract String i();

  @Nullable String jjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjj(
      int bbbbbbbbbbbbbb) {
    return null;
  }

  @Nullable String k;

  @Column(name = "l")
  @Nullable String l;

  @Setter @VisibleForTesting
  private InventoryServiceDirectory inventoryServiceDirectory = new Isd();

  @Id @GeneratedValue //
  private Integer id;
  private String name;
  @ModificationStamp private long lastEditedInstant; // A comment after the field.
}
