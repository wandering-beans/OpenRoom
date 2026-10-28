class Libtiff < Formula
  desc "TIFF library and utilities"
  homepage "https://libtiff.gitlab.io/libtiff/"
  url "https://download.osgeo.org/libtiff/tiff-4.7.2.tar.gz"
  mirror "https://ftp2.osuosl.org/pub/osgeo/download/libtiff/tiff-4.7.2.tar.gz"
  sha256 "672bd7d10aee4606171afb864f3570b83340f6a33e2c186dc0512f7145ffdf6a"
  license "libtiff"
  compatibility_version 1

  livecheck do
    url "https://download.osgeo.org/libtiff/"
    regex(/href=.*?tiff[._-]v?(\d+(?:\.\d+)+)\.t/i)
  end

  depends_on "jpeg-turbo"
  depends_on "webp"
  depends_on "xz"
  depends_on "zstd"

  on_linux do
    depends_on "zlib-ng-compat"
  end

  deny_network_access!

  def install
    args = %W[
      --disable-libdeflate
      --enable-webp
      --with-webp-include-dir=#{formula_opt_include("webp")}
      --with-webp-lib-dir=#{formula_opt_lib("webp")}
      --enable-zstd
      --enable-lzma
      --with-jpeg-include-dir=#{formula_opt_include("jpeg-turbo")}
      --with-jpeg-lib-dir=#{formula_opt_lib("jpeg-turbo")}
      --without-x
    ]
    system "./configure", *args, *std_configure_args
    system "make", "install"

    # Avoid rebuilding dependents that hard-code the prefix.
    inreplace lib/"pkgconfig/libtiff-4.pc", prefix, opt_prefix
  end

  test do
    (testpath/"test.c").write <<~C
      #include <tiffio.h>

      int main(int argc, char* argv[])
      {
        TIFF *out = TIFFOpen(argv[1], "w");
        TIFFSetField(out, TIFFTAG_IMAGEWIDTH, (uint32) 10);
        TIFFClose(out);
        return 0;
      }
    C
    system ENV.cc, "test.c", "-L#{lib}", "-ltiff", "-o", "test"
    system "./test", "test.tif"
    assert_match(/ImageWidth.*10/, shell_output("#{bin}/tiffdump test.tif"))
    (testpath/"test_webp.c").write <<~C
      #include <stdint.h>
      #include <tiffio.h>

      int main(int argc, char* argv[])
      {
        uint8_t rgb[16 * 16 * 3] = {0};
        TIFF *out = TIFFOpen(argv[1], "w");
        if (!out) return 1;
        TIFFSetField(out, TIFFTAG_IMAGEWIDTH, 16);
        TIFFSetField(out, TIFFTAG_IMAGELENGTH, 16);
        TIFFSetField(out, TIFFTAG_SAMPLESPERPIXEL, 3);
        TIFFSetField(out, TIFFTAG_BITSPERSAMPLE, 8);
        TIFFSetField(out, TIFFTAG_PHOTOMETRIC, PHOTOMETRIC_RGB);
        TIFFSetField(out, TIFFTAG_COMPRESSION, COMPRESSION_WEBP);
        TIFFSetField(out, TIFFTAG_ROWSPERSTRIP, 16);
        if (TIFFWriteEncodedStrip(out, 0, rgb, sizeof(rgb)) < 0) return 2;
        TIFFClose(out);
        return 0;
      }
    C
    system ENV.cc, "test_webp.c", "-L#{lib}", "-ltiff", "-o", "test_webp"
    system "./test_webp", "webp.tif"
    assert_match "Compression Scheme: WEBP", shell_output("#{bin}/tiffinfo webp.tif")
  end
end
