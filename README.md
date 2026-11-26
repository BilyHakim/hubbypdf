# HubbyPDF

Pembaca dan penyunting PDF lokal untuk Windows x64. HubbyPDF menggunakan Rust, kontrol Win32, Direct2D untuk menampilkan halaman, dan PDFium untuk membaca serta menyimpan PDF. Aplikasi merender halaman yang terlihat dan membatasi tiap bitmap hingga 4.096 piksel pada sisi terpanjang dan sekitar 9,5 juta piksel secara keseluruhan.

## Instalasi di Windows

Jalankan `HubbyPDF-0.1.1-x64.msi` pada Windows 10 atau 11 x64. Untuk memperbarui, tutup HubbyPDF lalu jalankan MSI yang sama. Installer memerlukan izin administrator dan memasang HubbyPDF ke Program Files, menambahkan shortcut ke Start Menu, serta mendaftarkannya sebagai pembuka PDF.

Untuk menjadikan HubbyPDF pembuka PDF default, buka **Settings → Apps → Default apps**, cari `.pdf`, lalu pilih **HubbyPDF**. Aplikasi dapat dihapus dari daftar aplikasi terinstal di Windows.

## Membuka aplikasi

Buka **HubbyPDF** dari Start Menu setelah instalasi, atau jalankan versi portabel di `target/HubbyPDF/HubbyPDF.exe`. Untuk memindahkan versi portabel, salin seluruh folder `target/HubbyPDF` agar DLL dan lisensi PDFium ikut terbawa.

- Klik **Tema** di kanan atas atau pilih **Tampilan → Tema gelap**. HubbyPDF mengingat pilihan tema dan menampilkan PDF dengan warna aslinya.
- Pilih **Bahasa → Bahasa Indonesia** atau **Bahasa → English**. HubbyPDF mengingat pilihan bahasa.
- Seret dan lepas PDF dari File Explorer ke jendela HubbyPDF, atau gunakan **File → Buka PDF** maupun **Ctrl+O**. Jika beberapa file dilepas sekaligus, HubbyPDF membuka PDF pertama dalam daftar. Anda juga bisa memberikan path PDF sebagai argumen.
- Pilih **File → PDF terakhir dibuka** untuk membuka kembali salah satu dari 10 PDF terakhir. Daftar disimpan secara lokal dan tetap tersedia setelah aplikasi ditutup.
- Gulir dengan **roda mouse**, **touchpad**, atau **panah atas/bawah** untuk membaca halaman secara berurutan. Seret bilah gulir di sisi kanan untuk berpindah cepat. Nomor halaman mengikuti posisi gulir. Gunakan **Page Up / Page Down** untuk langsung pindah halaman.
- Dalam mode **Pilih**, kursor berubah menjadi ikon tangan di atas tautan internal. Klik tautan di daftar isi untuk langsung menuju bagian yang dirujuk.
- Pilih zoom **25%–500%** atau **Pas jendela** di kanan bawah, atau gunakan **Ctrl+roda mouse**. **100%** berarti ukuran aktual PDF pada skala Windows. Gunakan **panah kiri/kanan** untuk menggeser tampilan saat halaman lebih lebar dari area baca.
- Klik nomor halaman di kiri bawah untuk membuka daftar dengan mini preview, lalu pilih halaman yang ingin dibuka.
- Klik **Teks**, lalu klik posisi pada halaman. Ketik teks Latin dasar berukuran 14 pt dan tekan **Enter** untuk menempatkannya. **Esc** membatalkan teks yang masih diketik.
- Klik **Teks**, **Sketsa**, atau **Tanda tangan** untuk memilih warna dari sampel warna di menu alat. Menu **Bentuk** memuat pilihan bentuk dan warna sekaligus. Tiap alat mengingat warna terakhirnya selama aplikasi terbuka.
- Buka **Bentuk**, pilih persegi panjang, elips, garis, atau panah, lalu seret di halaman. Untuk panah, titik lepas mouse menentukan letak ujung panah.
- **Sorot**, **Sketsa**, dan **Tanda tangan** menggunakan gerakan seret. Dengan **Impor PNG**, seret secara diagonal untuk mengatur posisi dan ukuran. Gambar yang sisi terpanjangnya melebihi 1.024 piksel diperkecil saat impor.
- Klik **Pilih**, lalu seret teks tambahan, sorotan, sketsa, atau tanda tangan yang dibuat dalam sesi ini. Goresan yang dibuat dalam satu penggunaan mode Tanda tangan ikut digeser bersama. Untuk memindahkan objek lagi, tetap gunakan dokumen yang sedang terbuka. Salinan PDF menyimpan posisi akhir sebagai konten halaman.
- Dalam mode **Pilih**, klik kolom teks, dropdown, atau checkbox pada formulir AcroForm. Tekan **Enter** untuk menerapkan teks atau **Esc** untuk membatalkannya. **Ctrl+Shift+S** menyimpan salinan dengan kolom yang tetap bisa diisi. Kalkulasi dan format otomatis yang bergantung pada JavaScript PDF belum didukung.
- Klik **Hapus**, lalu klik teks tambahan, sorotan, goresan sketsa, atau tanda tangan yang dibuat pada dokumen yang sedang terbuka. Klik salah satu goresan tanda tangan untuk menghapus seluruh tanda tangan. **Ctrl+Z** mengembalikan objek yang terakhir dihapus.
- **Ctrl+Z** membatalkan edit terakhir. **Esc** kembali ke mode Pilih. **Ctrl+Shift+S** menyimpan hasil sebagai file PDF terpisah.

Versi awal mendukung penambahan objek pada halaman. Penggantian teks asli, OCR, pembukaan PDF yang membutuhkan kata sandi, dan tanda tangan digital berbasis sertifikat berada di luar cakupan versi ini.

## Build

Gunakan Rust stable dan Visual Studio Build Tools dengan komponen **Desktop development with C++** dan Windows SDK. Dari folder proyek, jalankan:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File ./build.ps1 -Test
```

Skrip mengunduh PDFium Windows x64 pada build pertama, memeriksa SHA-256 arsip, dan menyalin aplikasi serta dependensi runtime ke `target/HubbyPDF`. Opsi `-Test` menjalankan pengujian PDF nyata yang memverifikasi render, teks, sorotan, sketsa, tanda tangan, simpan salinan, dan undo.

### Build installer

Ekstrak [WiX Toolset 3.14.1 binaries](https://github.com/wixtoolset/wix3/releases/tag/wix3141rtm) ke `target/wix314` atau tentukan folder bin WiX dengan parameter `-WixBin`. Build menggunakan `rc.exe` dari Windows SDK dan mengonversi `assets/hubbypdf.png` menjadi ikon Windows.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File ./build-installer.ps1 -Test
```

MSI dibuat di `target/installer`. Installer mengambil versi dari `Cargo.toml`. Naikkan versi tersebut saat merilis pembaruan.



